use super::assembled::Assembled;
use super::deferred::Deferred;
use super::events::PlayerEvents;
use super::fetcher::{Fetcher, ForeignFetcher};
use crate::analysis::{MediaTone, ToneSource};
use crate::api::TonePreparation;
use super::limits::{DeviceMemory, Evict, ImageWindow, Limits, PoolSettings};
use crate::api::{
    assemble, placeholder_for, CacheState, Placeholder, Assembly, Kind, MediaError, MediaItem, MediaState, MediaStats, Parts, PlaybackRoute,
    PlayerSlotFactory, SettingsStore, Subscription,
};
use crate::memory::InputLimits;
use crate::api::AbortOnDrop;
use crate::session::WatchdogConfig;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::runtime::{Builder, Handle, Runtime};

const WORKER_THREADS: usize = 2;

#[derive(uniffi::Record)]
pub struct OpenConfig {
    pub dir: String,
    pub available_bytes: u64,
    pub fetcher: Arc<dyn Fetcher>,
    pub settings: Arc<dyn SettingsStore>,
    pub slots: Option<Arc<dyn PlayerSlotFactory>>,
    pub device: DeviceMemory,
    pub defaults: Limits,
    pub pool: PoolSettings,
    #[uniffi(default = [])]
    pub namespace_headers: Vec<String>,
    #[uniffi(default = [])]
    pub image_windows: Vec<ImageWindow>,
    #[uniffi(default = None)]
    pub forward_headers: Option<Vec<String>>,
    #[uniffi(default = None)]
    pub evict_video: Option<Evict>,
    #[uniffi(default = None)]
    pub evict_image: Option<Evict>,
    #[uniffi(default = None)]
    pub evict_audio: Option<Evict>,
    #[uniffi(default = None)]
    pub memory_releaser: Option<Arc<dyn MemoryReleaser>>,
    #[uniffi(default = false)]
    pub silent: bool,
}

#[uniffi::export(with_foreign)]
pub trait MemoryReleaser: Send + Sync {
    fn drop_memory_caches(&self);
    fn shutdown(&self);
}

#[uniffi::export(with_foreign)]
pub trait StatsListener: Send + Sync {
    fn changed(&self, stats: MediaStats);
}

#[uniffi::export(with_foreign)]
pub trait ToneListener: Send + Sync {
    fn changed(&self, source: ToneSource);
}

#[uniffi::export(with_foreign)]
pub trait StateListener: Send + Sync {
    fn changed(&self, state: MediaState);
}

#[derive(uniffi::Object)]
pub struct MediaLayer {
    handle: Handle,
    runtime: Mutex<Option<Runtime>>,
    assembled: Arc<Deferred<Assembled>>,
    namespace_headers: Vec<String>,
    image_stats: Arc<super::image::ImageCounters>,
    releaser: Mutex<Option<Arc<dyn MemoryReleaser>>>,
}

impl MediaLayer {
    pub fn from_parts(runtime: Option<Runtime>, handle: Handle, parts: Parts, releaser: Option<Arc<dyn MemoryReleaser>>, namespace_headers: Vec<String>) -> Arc<MediaLayer> {
        let layer = MediaLayer::pending(runtime, handle.clone(), namespace_headers, releaser.clone());
        layer.assembled.install(Arc::new(Assembled::new(&handle, parts, releaser)));
        layer
    }

    pub(crate) fn assembling(runtime: Runtime, assembling: impl Future<Output = Result<Parts, MediaError>> + Send + 'static, releaser: Option<Arc<dyn MemoryReleaser>>, namespace_headers: Vec<String>) -> Arc<MediaLayer> {
        let handle = runtime.handle().clone();
        let layer = MediaLayer::pending(Some(runtime), handle.clone(), namespace_headers, releaser.clone());
        let assembled = layer.assembled.clone();
        handle.clone().spawn(async move {
            match assembling.await {
                Ok(parts) => assembled.install(Arc::new(Assembled::new(&handle, parts, releaser))),
                Err(error) => assembled.fail(error),
            }
        });
        layer
    }

    fn pending(runtime: Option<Runtime>, handle: Handle, namespace_headers: Vec<String>, releaser: Option<Arc<dyn MemoryReleaser>>) -> Arc<MediaLayer> {
        Arc::new(MediaLayer { handle, runtime: Mutex::new(runtime), assembled: Deferred::new(), namespace_headers, image_stats: super::image::ImageCounters::new(), releaser: Mutex::new(releaser) })
    }

    #[cfg(test)]
    pub(crate) fn queued_declarations(&self) -> usize {
        self.assembled.queued()
    }

    fn declare(&self, work: impl FnOnce(&Assembled) + Send + 'static) {
        self.assembled.run(work);
    }

    fn keep_wholes<'a>(&self, items: impl Iterator<Item = &'a MediaItem>) {
        if !self.assembled.is_pending() {
            return;
        }
        for item in items.filter(|item| item.strategy == crate::prefetch::CacheStrategy::Whole) {
            self.declare_latest(item.clone(), |item| format!("whole:{}", item.url), |assembled, item| assembled.stage.keep_whole(&item));
        }
    }

    fn declare_latest<P: Send + 'static>(&self, payload: P, name: fn(&P) -> String, work: impl FnOnce(&Assembled, P) + Send + 'static) {
        self.assembled.run_latest(payload, name, work);
    }

    pub(crate) async fn with_assembled<T: Send + 'static, Work: Future<Output = T> + Send + 'static>(&self, work: impl FnOnce(Arc<Assembled>) -> Work) -> Result<T, MediaError> {
        let assembled = self.assembled.wait().await?;
        AbortOnDrop::new(self.handle.spawn(work(assembled))).join().await.map_err(|error| MediaError::Unexpected { detail: error.to_string() })
    }

    async fn on_assembled<T: Send + 'static, Work: Future<Output = T> + Send + 'static>(&self, empty: T, work: impl FnOnce(Arc<Assembled>) -> Work) -> T {
        self.with_assembled(work).await.unwrap_or(empty)
    }
}

impl MediaLayer {
    fn teardown(&self) {
        let releaser = self.releaser.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        if let Some(releaser) = releaser {
            releaser.shutdown();
        }
        if let Some(runtime) = self.runtime.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
            runtime.shutdown_background();
        }
    }
}

impl Drop for MediaLayer {
    fn drop(&mut self) {
        self.teardown();
    }
}

#[uniffi::export]
impl MediaLayer {
    #[uniffi::constructor]
    pub fn open(config: OpenConfig) -> Result<Arc<MediaLayer>, MediaError> {
        let defaults = crate::limits::Limits::try_from(config.defaults)?;
        defaults.validate()?;
        let runtime = Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .thread_name("medialayer")
            .enable_all()
            .build()
            .map_err(|error| MediaError::Unexpected { detail: error.to_string() })?;
        let namespace_headers = config.namespace_headers.clone();
        let assembly = Assembly {
            available_bytes: config.available_bytes,
            dir: config.dir.into(),
            fetcher: Arc::new(ForeignFetcher::new(config.fetcher, InputLimits::default())),
            settings: config.settings,
            factory: config.slots,
            device: config.device.into(),
            defaults,
            pool: config.pool.into(),
            namespace_headers: config.namespace_headers,
            image_windows: config.image_windows.iter().map(Into::into).collect(),
            forward_headers: config.forward_headers,
            eviction_overrides: [
                (crate::cache::Kind::Stream, config.evict_video),
                (crate::cache::Kind::Image, config.evict_image),
                (crate::cache::Kind::Audio, config.evict_audio),
            ].into_iter().filter_map(|(kind, evict)| evict.map(|evict| (kind, evict.into()))).collect(),
            silent: config.silent,
            watchdog: WatchdogConfig::default(),
        };
        Ok(MediaLayer::assembling(runtime, assemble(assembly), config.memory_releaser, namespace_headers))
    }

    pub fn shutdown(&self) {
        self.teardown();
    }

    pub fn cache_key(&self, url: String, headers: HashMap<String, String>) -> String {
        crate::identity::namespaced_key(&url, &headers, &self.namespace_headers)
    }

    pub async fn limits(&self) -> Result<Limits, MediaError> {
        self.with_assembled(|assembled| async move { Limits::from(assembled.controls.limits().await) }).await
    }

    pub async fn set_limits(&self, limits: Limits) -> Result<(), MediaError> {
        let limits = crate::limits::Limits::try_from(limits)?;
        self.with_assembled(|assembled| async move { assembled.controls.set_limits(limits).await }).await?
    }

    pub async fn reset_limits(&self) -> Result<(), MediaError> {
        self.with_assembled(|assembled| async move { assembled.controls.reset_limits().await }).await?
    }

    pub async fn stats(&self) -> Result<MediaStats, MediaError> {
        self.with_assembled(|assembled| async move { assembled.controls.stats().await }).await
    }

    pub async fn clear(&self, kind: Option<Kind>) {
        self.on_assembled((), |assembled| async move { assembled.controls.clear(kind).await }).await
    }

    pub async fn purge(&self, url: String) {
        self.on_assembled((), |assembled| async move { assembled.controls.purge(&url).await }).await
    }

    pub async fn evict_now(&self) {
        self.on_assembled((), |assembled| async move { assembled.controls.evict_now().await }).await
    }

    pub fn subscribe_stats(&self, listener: Arc<dyn StatsListener>) -> Arc<Subscription> {
        Arc::new(self.assembled.hold(move |assembled| assembled.controls.subscribe_stats(Arc::new(move |stats| listener.changed(stats)))))
    }

    pub fn subscribe_state(&self, key: String, listener: Arc<dyn StateListener>) -> Arc<Subscription> {
        Arc::new(self.assembled.hold(move |assembled| assembled.places.subscribe(&key, Arc::new(move |state| listener.changed(state)))))
    }

    pub fn subscribe_tone(&self, source: MediaItem, listener: Arc<dyn ToneListener>) -> Arc<Subscription> {
        Arc::new(self.assembled.hold(move |assembled| assembled.tones.subscribe(&source, Arc::new(move |source| listener.changed(source)))))
    }

    pub async fn tone(&self, source: MediaItem, band: f32) -> Result<Option<MediaTone>, MediaError> {
        let assembled = self.assembled.wait().await?;
        Ok(assembled.tones.tone(&source, band).await?)
    }

    pub fn set_tone_preparation(&self, preparation: Arc<dyn TonePreparation>) {
        self.declare(move |assembled| assembled.tones.set_preparation(preparation));
    }

    pub async fn tone_pixels_needed(&self, source: MediaItem) -> bool {
        self.on_assembled(false, move |assembled| async move { assembled.tones.pixels_needed(&source).await }).await
    }

    pub async fn play_url(&self, source: MediaItem) -> Result<PlaybackRoute, MediaError> {
        let assembled = self.assembled.wait().await?;
        Ok(assembled.routes.play_url(&source))
    }

    pub async fn refresh_route(&self, source: MediaItem, route: PlaybackRoute) -> Result<PlaybackRoute, MediaError> {
        let assembled = self.assembled.wait().await?;
        Ok(assembled.routes.refresh(&source, &route))
    }

    pub async fn fetch_image(&self, url: String, headers: HashMap<String, String>) -> Result<crate::proxy::ImageFetchResult, MediaError> {
        self.with_assembled(|assembled| async move { assembled.routes.image_bytes(&url, &headers, InputLimits::default().response_bytes).await }).await?
    }

    pub fn image_stats(&self) -> Arc<super::image::ImageCounters> { self.image_stats.clone() }

    pub async fn is_cached(&self, source: MediaItem) -> bool {
        self.on_assembled(false, |assembled| async move { assembled.routes.is_cached(&source.url, &source.headers).await }).await
    }

    pub async fn placeholder(&self, source: MediaItem, ready: bool) -> Placeholder {
        let cache = if self.is_cached(source.clone()).await { CacheState::Cached } else { CacheState::Missing };
        placeholder_for(source, cache, ready)
    }

    pub fn item_state(&self, key: String, state: MediaState) {
        self.declare_latest(key, |key| format!("state:{key}"), move |assembled, key| {
            if state == MediaState::Ready { assembled.tones.refresh(&key); }
            assembled.places.item_state(&key, state);
        });
    }

    pub fn seek(&self, key: String, position_ms: u64) {
        self.declare_latest(key, |key| format!("seek:{key}"), move |assembled, key| assembled.stage.seek(&key, position_ms));
    }

    pub fn replace_window(&self, active: u32, sources: Vec<Option<MediaItem>>) {
        self.keep_wholes(sources.iter().flatten());
        self.declare_latest(sources, |_| "window".to_string(), move |assembled, sources| assembled.stage.replace_window(active as usize, sources));
    }

    pub fn move_window(&self, active: u32) {
        self.declare_latest((), |_| "window/move".to_string(), move |assembled, ()| assembled.stage.move_window(active as usize));
    }

    pub fn move_image_window(&self, name: String, active: u32) {
        self.declare_latest(name, |name| format!("image:{name}/move"), move |assembled, name| assembled.stage.move_image_window(&name, active as usize));
    }

    pub fn replace_image_window(&self, name: String, active: u32, pages: Vec<Vec<MediaItem>>) {
        self.keep_wholes(pages.iter().flatten());
        self.declare_latest((name, pages), |(name, _)| format!("image:{name}"), move |assembled, (name, pages)| assembled.stage.replace_image_window(&name, active as usize, pages));
    }

    pub fn set_playing(&self, key: String, playing: bool) {
        self.declare_latest(key, |key| format!("playing:{key}"), move |assembled, key| assembled.stage.set_playing(&key, playing));
    }

    pub fn set_peeking(&self, key: String, peeking: bool) {
        self.declare_latest(key, |key| format!("peeking:{key}"), move |assembled, key| assembled.stage.set_peeking(&key, peeking));
    }

    pub fn set_rate(&self, key: String, rate: f32) {
        self.declare_latest(key, |key| format!("rate:{key}"), move |assembled, key| assembled.stage.set_rate(&key, rate));
    }

    pub fn set_muted_all(&self, muted: bool) {
        self.declare_latest((), |_| "muted-all".to_string(), move |assembled, ()| assembled.stage.set_muted_all(muted));
    }

    pub fn set_muted(&self, key: String, muted: bool) {
        self.declare_latest(key, |key| format!("muted:{key}"), move |assembled, key| assembled.stage.set_muted(&key, muted));
    }

    pub fn visible(&self, source: MediaItem, visible: bool) {
        if visible {
            self.keep_wholes(std::iter::once(&source));
            self.assembled.run_opening(source, |source| source.url.clone(), |assembled, source| assembled.stage.visible(source, true));
        } else {
            self.assembled.run_closing(source, |source| source.url.clone(), |assembled, source| assembled.stage.visible(source, false));
        }
    }

    pub fn pin(&self, key: String) {
        self.declare_latest(key, |key| format!("pin:{key}"), move |assembled, key| assembled.stage.pin(&key));
    }

    pub fn unpin(&self, key: String) {
        self.declare_latest(key, |key| format!("pin:{key}"), move |assembled, key| assembled.stage.unpin(&key));
    }

    pub fn release_all(&self) {
        self.assembled.run_resetting(|assembled| assembled.stage.release_all());
    }

    pub fn player_events(&self) -> Arc<PlayerEvents> {
        PlayerEvents::new(self.assembled.clone())
    }
}
