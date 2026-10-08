use super::events::PlayerEvents;
use super::fetcher::{Fetcher, ForeignFetcher};
use super::limits::{DeviceMemory, Limits, PoolSettings};
use crate::api::{
    assemble, placeholder_for, CacheState, MemoryRelease, Placeholder, spawn_pulse, AbortOnDrop, Assembly, ControlPorts, Controls, Kind, MediaError, MediaItem, MediaState, MediaStats, Parts, PlaybackRoute,
    Places, PlayerSlotFactory, Routes, SettingsStore, Signals, Stage, Subscription,
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
    #[uniffi(default = None)]
    pub memory_releaser: Option<Arc<dyn MemoryReleaser>>,
}

#[uniffi::export(with_foreign)]
pub trait MemoryReleaser: Send + Sync {
    fn drop_memory_caches(&self);
}

struct ForeignRelease(Option<Arc<dyn MemoryReleaser>>);

impl MemoryRelease for ForeignRelease {
    fn drop_caches(&self) {
        if let Some(releaser) = &self.0 {
            releaser.drop_memory_caches();
        }
    }
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
    controls: Arc<Controls>,
    signals: Arc<Signals>,
    stage: Stage,
    places: Arc<Places>,
    routes: Arc<Routes>,
    _pulse: AbortOnDrop,
}

impl MediaLayer {
    pub fn from_parts(runtime: Option<Runtime>, handle: Handle, parts: Parts, releaser: Option<Arc<dyn MemoryReleaser>>) -> Arc<MediaLayer> {
        let ports = ControlPorts {
            cache: Arc::new(parts.cache),
            pool: Arc::new(parts.stage.clone()),
            prefetch: Arc::new(parts.prefetcher.clone()),
            settings: parts.settings,
            ledger: parts.ledger.clone(),
            device: parts.device,
            transport: Arc::new(parts.proxy.clone()),
            pool_settings: parts.pool,
        };
        let controls = Arc::new(Controls::new(handle.clone(), ports, parts.defaults));
        let signals = Arc::new(Signals::new(
            &handle,
            Arc::new(parts.stage.clone()),
            Arc::new(parts.prefetcher),
            Arc::new(parts.stage.clone()),
            Arc::new(ForeignRelease(releaser)),
            parts.ledger,
            parts.clock,
        ));
        let warm = controls.clone();
        handle.spawn(async move {
            warm.limits().await;
        });
        let pulse = spawn_pulse(&handle, Arc::downgrade(&controls), parts.stats_interval);
        Arc::new(MediaLayer {
            handle,
            runtime: Mutex::new(runtime),
            controls,
            signals,
            stage: parts.stage,
            places: parts.places,
            routes: parts.routes,
            _pulse: pulse,
        })
    }

    async fn on_runtime<T: Send + 'static>(&self, work: impl Future<Output = T> + Send + 'static) -> T {
        self.handle.spawn(work).await.expect("a media task panicked")
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
        let runtime = Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .thread_name("medialayer")
            .enable_all()
            .build()
            .map_err(|error| MediaError::Unexpected { detail: error.to_string() })?;
        let handle = runtime.handle().clone();
        let releaser = config.memory_releaser;
        let assembly = Assembly {
            dir: config.dir.into(),
            fetcher: Arc::new(ForeignFetcher::new(config.fetcher, InputLimits::default())),
            settings: config.settings,
            factory: config.slots,
            device: config.device.into(),
            defaults: config.defaults.into(),
            pool: config.pool.into(),
            namespace_headers: config.namespace_headers,
            watchdog: WatchdogConfig::default(),
            stats_interval: STATS_INTERVAL,
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        runtime.spawn(async move {
            let _ = sender.send(assemble(assembly).await);
        });
        let parts = receiver.recv().map_err(|error| MediaError::Unexpected { detail: error.to_string() })??;
        Ok(MediaLayer::from_parts(Some(runtime), handle, parts, releaser))
    }

    pub async fn limits(&self) -> Limits {
        let controls = self.controls.clone();
        self.on_runtime(async move { controls.limits().await.into() }).await
    }

    pub async fn set_limits(&self, limits: Limits) -> Result<(), MediaError> {
        let controls = self.controls.clone();
        self.on_runtime(async move { controls.set_limits(limits.into()).await }).await
    }

    pub async fn reset_limits(&self) -> Result<(), MediaError> {
        let controls = self.controls.clone();
        self.on_runtime(async move { controls.reset_limits().await }).await
    }

    pub async fn stats(&self) -> MediaStats {
        let controls = self.controls.clone();
        self.on_runtime(async move { controls.stats().await }).await
    }

    pub async fn clear(&self, kind: Option<Kind>) {
        let controls = self.controls.clone();
        self.on_runtime(async move { controls.clear(kind).await }).await
    }

    pub async fn purge(&self, url: String) {
        let controls = self.controls.clone();
        self.on_runtime(async move { controls.purge(&url).await }).await
    }

    pub async fn evict_now(&self) {
        let controls = self.controls.clone();
        self.on_runtime(async move { controls.evict_now().await }).await
    }

    pub fn subscribe_stats(&self, listener: Arc<dyn StatsListener>) -> Arc<Subscription> {
        Arc::new(self.controls.subscribe_stats(Arc::new(move |stats| listener.changed(stats))))
    }

    pub fn subscribe_state(&self, key: String, listener: Arc<dyn StateListener>) -> Arc<Subscription> {
        Arc::new(self.places.subscribe(&key, Arc::new(move |state| listener.changed(state))))
    }

    pub fn play_url(&self, source: MediaItem) -> PlaybackRoute {
        self.routes.play_url(&source)
    }

    pub fn refresh_route(&self, source: MediaItem, route: PlaybackRoute) -> PlaybackRoute {
        self.routes.refresh(&source, &route)
    }

    pub async fn fetch_image(&self, url: String, headers: HashMap<String, String>) -> Result<Vec<u8>, MediaError> {
        let routes = self.routes.clone();
        self.on_runtime(async move { routes.image_bytes(&url, &headers, InputLimits::default().response_bytes).await }).await
    }

    pub async fn is_cached(&self, source: MediaItem) -> bool {
        let routes = self.routes.clone();
        self.on_runtime(async move { routes.is_cached(&source.url, &source.headers).await }).await
    }

    pub async fn placeholder(&self, source: MediaItem, ready: bool) -> Placeholder {
        let cache = if self.is_cached(source.clone()).await { CacheState::Cached } else { CacheState::Missing };
        placeholder_for(source, cache, ready)
    }

    pub fn item_state(&self, key: String, state: MediaState) {
        self.places.item_state(&key, state);
    }

    pub fn seek(&self, key: String, position_ms: u64) {
        self.stage.seek(&key, position_ms);
    }

    pub fn set_window(&self, active: u32, sources: Vec<Option<MediaItem>>) {
        self.stage.set_window(active as usize, sources);
    }

    pub fn set_playing(&self, key: String, playing: bool) {
        self.stage.set_playing(&key, playing);
    }

    pub fn set_muted_all(&self, muted: bool) {
        self.stage.set_muted_all(muted);
    }

    pub fn set_muted(&self, key: String, muted: bool) {
        self.stage.set_muted(&key, muted);
    }

    pub fn visible(&self, source: MediaItem, visible: bool) {
        self.stage.visible(source, visible);
    }

    pub fn pin(&self, key: String) {
        self.stage.pin(&key);
    }

    pub fn unpin(&self, key: String) {
        self.stage.unpin(&key);
    }

    pub fn release_all(&self) {
        self.stage.release_all();
    }

    pub fn player_events(&self) -> Arc<PlayerEvents> {
        PlayerEvents::new(self.stage.clone(), self.signals.clone())
    }
}
