use super::assembled::Assembled;
use super::deferred::Deferred;
use super::events::PlayerEvents;
use super::fetcher::{Fetcher, ForeignFetcher};
use super::limits::{DeviceMemory, ImageWindow, Limits, PoolSettings};
use crate::api::{
    assemble, placeholder_for, CacheState, Placeholder, Assembly, Kind, MediaError, MediaItem, MediaState, MediaStats, Parts, PlaybackRoute,
    PlayerSlotFactory, SettingsStore, Subscription,
};
use crate::memory::InputLimits;
use crate::session::WatchdogConfig;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::{Builder, Handle, Runtime};

const WORKER_THREADS: usize = 2;
const STATS_INTERVAL: Duration = Duration::from_millis(250);

#[derive(uniffi::Record)]
pub struct OpenConfig {
    pub dir: String,
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
    pub memory_releaser: Option<Arc<dyn MemoryReleaser>>,
    #[uniffi(default = false)]
    pub silent: bool,
}

#[uniffi::export(with_foreign)]
pub trait MemoryReleaser: Send + Sync {
    fn drop_memory_caches(&self);
}

#[uniffi::export(with_foreign)]
pub trait StatsListener: Send + Sync {
    fn changed(&self, stats: MediaStats);
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
}

impl MediaLayer {
    pub fn from_parts(runtime: Option<Runtime>, handle: Handle, parts: Parts, releaser: Option<Arc<dyn MemoryReleaser>>) -> Arc<MediaLayer> {
        let layer = MediaLayer::pending(runtime, handle.clone());
        layer.assembled.install(Arc::new(Assembled::new(&handle, parts, releaser)));
        layer
    }

    fn assembling(runtime: Runtime, assembling: impl Future<Output = Result<Parts, MediaError>> + Send + 'static, releaser: Option<Arc<dyn MemoryReleaser>>) -> Arc<MediaLayer> {
        let handle = runtime.handle().clone();
        let layer = MediaLayer::pending(Some(runtime), handle.clone());
        let assembled = layer.assembled.clone();
        handle.clone().spawn(async move {
            match assembling.await {
                Ok(parts) => assembled.install(Arc::new(Assembled::new(&handle, parts, releaser))),
                Err(error) => assembled.fail(error),
            }
        });
        layer
    }

    fn pending(runtime: Option<Runtime>, handle: Handle) -> Arc<MediaLayer> {
        Arc::new(MediaLayer { handle, runtime: Mutex::new(runtime), assembled: Deferred::new() })
    }

    fn declare(&self, work: impl FnOnce(&Assembled) + Send + 'static) {
        self.assembled.run(work);
    }

    async fn with_assembled<T: Send + 'static, Work: Future<Output = T> + Send + 'static>(&self, work: impl FnOnce(Arc<Assembled>) -> Work) -> Result<T, MediaError> {
        let assembled = self.assembled.wait().await?;
        Ok(self.handle.spawn(work(assembled)).await.expect("a media task panicked"))
    }

    async fn on_assembled<T: Send + 'static, Work: Future<Output = T> + Send + 'static>(&self, work: impl FnOnce(Arc<Assembled>) -> Work) -> T {
        self.with_assembled(work).await.expect("the media layer failed to assemble")
    }
}

impl Drop for MediaLayer {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
            runtime.shutdown_background();
        }
    }
}

#[uniffi::export]
impl MediaLayer {
    #[uniffi::constructor]
    pub fn open(config: OpenConfig) -> Result<Arc<MediaLayer>, MediaError> {
        let defaults: crate::limits::Limits = config.defaults.into();
        defaults.validate()?;
        let runtime = Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .thread_name("medialayer")
            .enable_all()
            .build()
            .map_err(|error| MediaError::Unexpected { detail: error.to_string() })?;
        let assembly = Assembly {
            dir: config.dir.into(),
            fetcher: Arc::new(ForeignFetcher::new(config.fetcher, InputLimits::default())),
            settings: config.settings,
            factory: config.slots,
            device: config.device.into(),
            defaults,
            pool: config.pool.into(),
            namespace_headers: config.namespace_headers,
            image_windows: config.image_windows.iter().map(Into::into).collect(),
            silent: config.silent,
            watchdog: WatchdogConfig::default(),
            stats_interval: STATS_INTERVAL,
        };
        Ok(MediaLayer::assembling(runtime, assemble(assembly), config.memory_releaser))
    }

    pub async fn limits(&self) -> Limits {
        self.on_assembled(|assembled| async move { assembled.controls.limits().await.into() }).await
    }

    pub async fn set_limits(&self, limits: Limits) -> Result<(), MediaError> {
        self.with_assembled(|assembled| async move { assembled.controls.set_limits(limits.into()).await }).await?
    }

    pub async fn reset_limits(&self) -> Result<(), MediaError> {
        self.with_assembled(|assembled| async move { assembled.controls.reset_limits().await }).await?
    }

    pub async fn stats(&self) -> MediaStats {
        self.on_assembled(|assembled| async move { assembled.controls.stats().await }).await
    }

    pub async fn clear(&self, kind: Option<Kind>) {
        self.on_assembled(|assembled| async move { assembled.controls.clear(kind).await }).await
    }

    pub async fn purge(&self, url: String) {
        self.on_assembled(|assembled| async move { assembled.controls.purge(&url).await }).await
    }

    pub async fn evict_now(&self) {
        self.on_assembled(|assembled| async move { assembled.controls.evict_now().await }).await
    }

    pub fn subscribe_stats(&self, listener: Arc<dyn StatsListener>) -> Arc<Subscription> {
        Arc::new(self.assembled.hold(move |assembled| assembled.controls.subscribe_stats(Arc::new(move |stats| listener.changed(stats)))))
    }

    pub fn subscribe_state(&self, key: String, listener: Arc<dyn StateListener>) -> Arc<Subscription> {
        Arc::new(self.assembled.hold(move |assembled| assembled.places.subscribe(&key, Arc::new(move |state| listener.changed(state)))))
    }

    pub async fn play_url(&self, source: MediaItem) -> Result<PlaybackRoute, MediaError> {
        let assembled = self.assembled.wait().await?;
        Ok(assembled.routes.play_url(&source))
    }

    pub async fn refresh_route(&self, source: MediaItem, route: PlaybackRoute) -> Result<PlaybackRoute, MediaError> {
        let assembled = self.assembled.wait().await?;
        Ok(assembled.routes.refresh(&source, &route))
    }

    pub async fn fetch_image(&self, url: String, headers: HashMap<String, String>) -> Result<Vec<u8>, MediaError> {
        self.with_assembled(|assembled| async move { assembled.routes.image_bytes(&url, &headers, InputLimits::default().response_bytes).await }).await?
    }

    pub async fn is_cached(&self, source: MediaItem) -> bool {
        self.on_assembled(|assembled| async move { assembled.routes.is_cached(&source.url, &source.headers).await }).await
    }

    pub async fn placeholder(&self, source: MediaItem, ready: bool) -> Placeholder {
        let cache = if self.is_cached(source.clone()).await { CacheState::Cached } else { CacheState::Missing };
        placeholder_for(source, cache, ready)
    }

    pub fn item_state(&self, key: String, state: MediaState) {
        self.declare(move |assembled| assembled.places.item_state(&key, state));
    }

    pub fn seek(&self, key: String, position_ms: u64) {
        self.declare(move |assembled| assembled.stage.seek(&key, position_ms));
    }

    pub fn replace_window(&self, active: u32, sources: Vec<Option<MediaItem>>) {
        self.declare(move |assembled| assembled.stage.replace_window(active as usize, sources));
    }

    pub fn move_window(&self, active: u32) {
        self.declare(move |assembled| assembled.stage.move_window(active as usize));
    }

    pub fn move_image_window(&self, name: String, active: u32) {
        self.declare(move |assembled| assembled.stage.move_image_window(&name, active as usize));
    }

    pub fn replace_image_window(&self, name: String, active: u32, pages: Vec<Vec<MediaItem>>) {
        self.declare(move |assembled| assembled.stage.replace_image_window(&name, active as usize, pages));
    }

    pub fn set_playing(&self, key: String, playing: bool) {
        self.declare(move |assembled| assembled.stage.set_playing(&key, playing));
    }

    pub fn set_peeking(&self, key: String, peeking: bool) {
        self.declare(move |assembled| assembled.stage.set_peeking(&key, peeking));
    }

    pub fn set_rate(&self, key: String, rate: f32) {
        self.declare(move |assembled| assembled.stage.set_rate(&key, rate));
    }

    pub fn set_muted_all(&self, muted: bool) {
        self.declare(move |assembled| assembled.stage.set_muted_all(muted));
    }

    pub fn set_muted(&self, key: String, muted: bool) {
        self.declare(move |assembled| assembled.stage.set_muted(&key, muted));
    }

    pub fn visible(&self, source: MediaItem, visible: bool) {
        self.declare(move |assembled| assembled.stage.visible(source, visible));
    }

    pub fn pin(&self, key: String) {
        self.declare(move |assembled| assembled.stage.pin(&key));
    }

    pub fn unpin(&self, key: String) {
        self.declare(move |assembled| assembled.stage.unpin(&key));
    }

    pub fn release_all(&self) {
        self.declare(|assembled| assembled.stage.release_all());
    }

    pub fn player_events(&self) -> Arc<PlayerEvents> {
        PlayerEvents::new(self.assembled.clone())
    }
}
