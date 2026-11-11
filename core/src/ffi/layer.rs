use super::assembled::Assembled;
use super::events::PlayerEvents;
use super::fetcher::{Fetcher, ForeignFetcher};
use super::ingress::{run, Answer, Declared, Admitted, IngressLimits, Finished, OwnerKind, Pause, InputQueue, Query, Subscribing, WindowKind};
use super::layer_life::LayerLife;
use super::limits::{DeviceMemory, Evict, ImageWindow, Limits, PoolSettings};
use super::owned::{CallHold, MediaPlace, MediaWholeDeclaration, MediaWindow, PlaybackSuspension};
use super::ready::Ready;
use crate::analysis::MediaTone;
use crate::api::TonePreparation;
use crate::api::{
    assemble, placeholder_for, Placeholder, Assembly, Kind, MediaError, MediaItem, MediaState, MediaStats, Parts, PlaybackRoute,
    PlayerSlotFactory, SettingsStore, Subscription,
};
use crate::memory::InputLimits;
use crate::api::AbortOnDrop;
use crate::session::WatchdogConfig;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::sync::watch;

const WORKER_THREADS: usize = 2;

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
    #[uniffi(default = None)]
    pub ingress: Option<IngressLimits>,
}

#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait MemoryReleaser: Send + Sync {
    fn drop_memory_caches(&self);
    async fn shutdown(&self);
}

#[uniffi::export(with_foreign)]
pub trait StatsListener: Send + Sync {
    fn changed(&self, stats: MediaStats);
    fn failed(&self, error: MediaError);
}

#[uniffi::export(with_foreign)]
pub trait ToneListener: Send + Sync {
    fn changed(&self, tone: Option<MediaTone>);
    fn failed(&self, error: MediaError);
}

#[uniffi::export(with_foreign)]
pub trait WindowReadyListener: Send + Sync {
    fn changed(&self, ready: bool);
}

#[uniffi::export(with_foreign)]
pub trait StateListener: Send + Sync {
    fn changed(&self, state: MediaState);
    fn failed(&self, error: MediaError);
}


pub(super) struct Closing {
    handle: Handle,
    life: Mutex<LayerLife>,
    pub(super) ready: Arc<Ready<Assembled>>,
    pub(super) queue: Arc<InputQueue>,
    executor: Mutex<Option<AbortOnDrop>>,
    runtime: Mutex<Option<Runtime>>,
    releaser: Mutex<Option<Arc<dyn MemoryReleaser>>>,
    assembling: Mutex<Option<AbortOnDrop>>,
    late: Mutex<Option<Arc<Assembled>>>,
    closed: watch::Sender<bool>,
}

impl Closing {
    fn life(&self) -> MutexGuard<'_, LayerLife> {
        self.life.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn shutdown(self: &Arc<Closing>) {
        if !self.life().shutdown() {
            return;
        }
        let retired = self.ready.close();
        self.queue.end(MediaError::Closed);
        let closing = self.clone();
        self.handle.spawn(async move { closing.clean(retired).await });
    }

    /// An overflow of native events cannot be dropped silently: observers get the error and the layer closes.
    pub(super) fn overflowed(self: &Arc<Closing>, error: MediaError) {
        self.queue.end(error);
        self.shutdown();
    }

    async fn clean(&self, retired: Option<Arc<Assembled>>) {
        let assembling = self.assembling.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(mut assembling) = assembling {
            assembling.abort();
            let _ = assembling.join().await;
        }
        let executor = self.executor.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(mut executor) = executor {
            let _ = executor.join().await;
        }
        let late = self.late.lock().unwrap_or_else(PoisonError::into_inner).take();
        for assembled in retired.into_iter().chain(late) {
            assembled.close().await;
        }
        let releaser = self.releaser.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(releaser) = releaser {
            releaser.shutdown().await;
        }
        self.life().cleaned();
        self.closed.send_replace(true);
        let runtime = self.runtime.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(runtime) = runtime {
            std::thread::Builder::new().name("medialayer-closer".to_string()).spawn(move || runtime.shutdown_background()).ok();
        }
    }
}

#[derive(uniffi::Object)]
pub struct MediaLayer {
    handle: Handle,
    closing: Arc<Closing>,
    namespace_headers: Vec<String>,
    image_stats: Arc<super::image::ImageCounters>,
    next_owner: AtomicU64,
    controls: tokio::sync::Mutex<()>,
}

impl MediaLayer {
    /// With an external `Handle` the caller keeps the runtime alive until `await_closed` returns.
    pub fn from_parts(runtime: Option<Runtime>, handle: Handle, parts: Parts, releaser: Option<Arc<dyn MemoryReleaser>>, namespace_headers: Vec<String>) -> Arc<MediaLayer> {
        let layer = MediaLayer::pending(runtime, handle.clone(), namespace_headers, releaser.clone(), IngressLimits::default(), Pause::new());
        if layer.closing.life().installed() {
            let _ = layer.closing.ready.install(Arc::new(Assembled::new(&handle, parts, releaser)));
        }
        layer
    }

    pub(crate) fn assembling(runtime: Runtime, assembling: impl Future<Output = Result<Parts, MediaError>> + Send + 'static, releaser: Option<Arc<dyn MemoryReleaser>>, namespace_headers: Vec<String>, limits: IngressLimits, pause: Pause) -> Arc<MediaLayer> {
        let handle = runtime.handle().clone();
        let layer = MediaLayer::pending(Some(runtime), handle.clone(), namespace_headers, releaser.clone(), limits, pause);
        let (closing, ready) = (layer.closing.clone(), layer.closing.ready.clone());
        let mut slot = layer.closing.assembling.lock().unwrap_or_else(PoisonError::into_inner);
        let task = handle.clone().spawn(async move {
            match assembling.await {
                Ok(parts) => {
                    let is_installed = closing.life().installed();
                    if is_installed {
                        let rejected = ready.install(Arc::new(Assembled::new(&handle, parts, releaser)));
                        *closing.late.lock().unwrap_or_else(PoisonError::into_inner) = rejected;
                    }
                }
                Err(error) => {
                    if closing.life().failed() {
                        ready.fail(error);
                    }
                }
            }
            let finished = closing.assembling.lock().unwrap_or_else(PoisonError::into_inner).take();
            drop(finished);
        });
        *slot = Some(AbortOnDrop::new(task));
        drop(slot);
        layer
    }

    fn pending(runtime: Option<Runtime>, handle: Handle, namespace_headers: Vec<String>, releaser: Option<Arc<dyn MemoryReleaser>>, limits: IngressLimits, pause: Pause) -> Arc<MediaLayer> {
        let ready = Ready::new();
        let queue = InputQueue::new(limits);
        let executor = AbortOnDrop::new(handle.spawn(run(queue.clone(), ready.clone(), pause)));
        let closing = Arc::new(Closing {
            handle: handle.clone(),
            life: Mutex::new(LayerLife::new()),
            ready,
            queue,
            executor: Mutex::new(Some(executor)),
            runtime: Mutex::new(runtime),
            releaser: Mutex::new(releaser),
            assembling: Mutex::new(None),
            late: Mutex::new(None),
            closed: watch::channel(false).0,
        });
        Arc::new(MediaLayer { handle, closing, namespace_headers, image_stats: super::image::ImageCounters::new(), next_owner: AtomicU64::new(1), controls: tokio::sync::Mutex::new(()) })
    }

    #[cfg(test)]
    pub(crate) fn owns_runtime(&self) -> bool {
        self.closing.runtime.lock().unwrap_or_else(PoisonError::into_inner).is_some()
    }

    #[cfg(test)]
    pub(crate) fn queue(&self) -> &Arc<InputQueue> {
        &self.closing.queue
    }

    #[cfg(test)]
    pub(super) fn closing_ref(&self) -> std::sync::Weak<Closing> {
        Arc::downgrade(&self.closing)
    }

    pub(crate) fn build(config: OpenConfig, pause: Pause) -> Result<Arc<MediaLayer>, MediaError> {
        let defaults = crate::limits::Limits::try_from(config.defaults)?;
        defaults.validate()?;
        let ingress = config.ingress.unwrap_or_default();
        ingress.validate()?;
        let runtime = Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .thread_name("medialayer")
            .enable_all()
            .build()
            .map_err(|error| MediaError::Unexpected { detail: error.to_string() })?;
        let namespace_headers = config.namespace_headers.clone();
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
            forward_headers: config.forward_headers,
            eviction_overrides: [
                (crate::cache::Kind::Stream, config.evict_video),
                (crate::cache::Kind::Image, config.evict_image),
                (crate::cache::Kind::Audio, config.evict_audio),
            ].into_iter().filter_map(|(kind, evict)| evict.map(|evict| (kind, evict.into()))).collect(),
            silent: config.silent,
            watchdog: WatchdogConfig::default(),
        };
        Ok(MediaLayer::assembling(runtime, assemble(assembly), config.memory_releaser, namespace_headers, ingress, pause))
    }

    fn open_window(&self, kind: WindowKind) -> Result<Arc<MediaWindow>, MediaError> {
        let owner = self.owner();
        self.closing.queue.open(owner, OwnerKind::Window, Declared::WindowOpen { owner, kind: kind.clone() })?;
        Ok(Arc::new(MediaWindow::new(self.closing.queue.clone(), owner, kind)))
    }

    fn owner(&self) -> u64 {
        self.next_owner.fetch_add(1, Ordering::Relaxed)
    }

    fn subscription(&self, owner: u64) -> Arc<Subscription> {
        let queue = self.closing.queue.clone();
        Arc::new(Subscription::cancelling(move || queue.release(owner)))
    }

    fn subscribe(&self, subscribing: Subscribing) -> Result<Arc<Subscription>, MediaError> {
        let owner = self.owner();
        self.closing.queue.open(owner, OwnerKind::Subscription, Declared::Subscribe { owner, subscribing })?;
        Ok(self.subscription(owner))
    }

    fn submit(&self, command: Declared) -> Result<(), MediaError> {
        self.closing.queue.submit(command)
    }

    async fn until_closed<T>(&self, work: impl Future<Output = T>) -> Result<T, MediaError> {
        let mut ended = self.closing.queue.ended();
        tokio::select! {
            output = work => Ok(output),
            _ = ended.wait_for(|ended| *ended) => Err(self.closing.queue.end_error().unwrap_or(MediaError::Closed)),
        }
    }

    /// A request ordered behind every accepted declaration. The executor fixes its logical result at that point.
    async fn ask(&self, query: Query) -> Result<(Admitted, Answer), MediaError> {
        let (held, answer) = self.closing.queue.query(query)?;
        let answer = self.until_closed(answer).await?.map_err(|_| MediaError::Closed)?;
        Ok((held, answer))
    }

    async fn route(&self, query: Query) -> Result<PlaybackRoute, MediaError> {
        match self.ask(query).await?.1 {
            Answer::Route(route) => Ok(route),
            Answer::Failed(error) => Err(error),
            _ => Err(MediaError::Unexpected { detail: "the executor answered with another result".to_string() }),
        }
    }

    /// IO that the executor started from its snapshot. The work stays counted until it ends or the caller cancels it.
    async fn outcome(&self, query: Query) -> Result<Finished, MediaError> {
        let (held, answer) = self.ask(query).await?;
        match answer {
            Answer::Running(mut task) => {
                let output = self.until_closed(task.join()).await?;
                drop(held);
                output.map_err(|error| MediaError::Unexpected { detail: error.to_string() })
            }
            Answer::Failed(error) => Err(error),
            _ => Err(MediaError::Unexpected { detail: "the executor answered with another result".to_string() }),
        }
    }

    /// Controls keep their own bounded order and never wait for media events.
    pub(crate) async fn controlled<T: Send + 'static, Work: Future<Output = T> + Send + 'static>(&self, work: impl FnOnce(Arc<Assembled>) -> Work) -> Result<T, MediaError> {
        let held = self.closing.queue.hold()?;
        let assembled = self.until_closed(self.closing.ready.wait()).await??;
        let _turn = self.until_closed(self.controls.lock()).await?;
        self.run_held(held, assembled, work).await
    }

    async fn run_held<T: Send + 'static, Work: Future<Output = T> + Send + 'static>(&self, held: Admitted, assembled: Arc<Assembled>, work: impl FnOnce(Arc<Assembled>) -> Work) -> Result<T, MediaError> {
        let mut task = AbortOnDrop::new(self.handle.spawn(work(assembled)));
        let output = self.until_closed(task.join()).await?;
        drop(held);
        output.map_err(|error| MediaError::Unexpected { detail: error.to_string() })
    }
}

impl Drop for MediaLayer {
    fn drop(&mut self) {
        self.closing.shutdown();
    }
}

#[uniffi::export]
impl MediaLayer {
    #[uniffi::constructor]
    pub fn open(config: OpenConfig) -> Result<Arc<MediaLayer>, MediaError> {
        MediaLayer::build(config, Pause::new())
    }

    pub fn shutdown(&self) {
        self.closing.shutdown();
    }

    pub async fn await_closed(&self) {
        self.closing.shutdown();
        let mut closed = self.closing.closed.subscribe();
        let _ = closed.wait_for(|closed| *closed).await;
    }

    pub fn cache_key(&self, url: String, headers: HashMap<String, String>) -> String {
        crate::identity::namespaced_key(&crate::api::clean_url(&url), &headers, &self.namespace_headers)
    }

    pub async fn limits(&self) -> Result<Limits, MediaError> {
        self.controlled(|assembled| async move { Limits::from(assembled.controls.limits().await) }).await
    }

    pub async fn set_limits(&self, limits: Limits) -> Result<(), MediaError> {
        let limits = crate::limits::Limits::try_from(limits)?;
        self.controlled(|assembled| async move { assembled.controls.set_limits(limits).await }).await?
    }

    pub async fn reset_limits(&self) -> Result<(), MediaError> {
        self.controlled(|assembled| async move { assembled.controls.reset_limits().await }).await?
    }

    pub async fn stats(&self) -> Result<MediaStats, MediaError> {
        let held = self.closing.queue.hold()?;
        let assembled = self.until_closed(self.closing.ready.wait()).await??;
        let _turn = self.until_closed(self.controls.lock()).await?;
        let pool = match self.ask(Query::Pool).await?.1 {
            Answer::Pool(pool) => pool,
            Answer::Failed(error) => return Err(error),
            _ => return Err(MediaError::Unexpected { detail: "the executor answered with another result".to_string() }),
        };
        self.run_held(held, assembled, move |assembled| async move { assembled.controls.stats_with(pool).await }).await
    }

    pub async fn clear(&self, kind: Option<Kind>) -> Result<(), MediaError> {
        self.controlled(|assembled| async move { assembled.controls.clear(kind).await }).await
    }

    pub async fn reset_user_data(&self) -> Result<(), MediaError> {
        self.controlled(|assembled| async move { assembled.controls.reset_user_data().await }).await?
    }

    pub async fn purge(&self, url: String) -> Result<(), MediaError> {
        let url = crate::api::clean_url(&url);
        self.controlled(|assembled| async move { assembled.controls.purge(&url).await }).await
    }

    pub async fn evict_now(&self) -> Result<(), MediaError> {
        self.controlled(|assembled| async move { assembled.controls.evict_now().await }).await
    }

    pub fn subscribe_stats(&self, listener: Arc<dyn StatsListener>) -> Result<Arc<Subscription>, MediaError> {
        self.subscribe(Subscribing::Stats { listener })
    }

    pub fn subscribe_state(&self, key: String, listener: Arc<dyn StateListener>) -> Result<Arc<Subscription>, MediaError> {
        self.subscribe(Subscribing::State { key, listener })
    }

    pub fn subscribe_tone(&self, source: MediaItem, band: f32, listener: Arc<dyn ToneListener>) -> Result<Arc<Subscription>, MediaError> {
        self.subscribe(Subscribing::Tone { item: source, band, listener })
    }

    pub async fn tone(&self, source: MediaItem, band: f32) -> Result<Option<MediaTone>, MediaError> {
        match self.outcome(Query::Tone(source, band)).await? {
            Finished::Tone(tone) => Ok(tone?),
            _ => Err(MediaError::Unexpected { detail: "the executor answered with another result".to_string() }),
        }
    }

    pub fn set_tone_preparation(&self, preparation: Arc<dyn TonePreparation>) -> Result<(), MediaError> {
        self.submit(Declared::Preparation(preparation))
    }

    pub async fn tone_pixels_needed(&self, source: MediaItem) -> bool {
        matches!(self.outcome(Query::Pixels(source)).await, Ok(Finished::Pixels(true)))
    }

    pub async fn play_url(&self, source: MediaItem) -> Result<PlaybackRoute, MediaError> {
        self.route(Query::PlayUrl(source)).await
    }

    pub async fn refresh_route(&self, source: MediaItem, route: PlaybackRoute) -> Result<PlaybackRoute, MediaError> {
        self.route(Query::Refresh(source, route)).await
    }

    pub async fn fetch_image(&self, url: String, headers: HashMap<String, String>) -> Result<crate::proxy::ImageFetchResult, MediaError> {
        match self.outcome(Query::Image { url, headers }).await? {
            Finished::Image(result) => result,
            _ => Err(MediaError::Unexpected { detail: "the executor answered with another result".to_string() }),
        }
    }

    pub async fn reserve_decode(&self, bytes: u64) -> Result<Arc<super::image::DecodeHold>, MediaError> {
        match self.outcome(Query::Decode(bytes)).await? {
            Finished::Decode(reserved) => Ok(Arc::new(super::image::DecodeHold::new(reserved?))),
            _ => Err(MediaError::Unexpected { detail: "the executor answered with another result".to_string() }),
        }
    }

    pub async fn reserve_capture(&self, bytes: u64) -> Result<Arc<super::image::DecodeHold>, MediaError> {
        match self.outcome(Query::Capture(bytes)).await? {
            Finished::Capture(reserved) => Ok(Arc::new(super::image::DecodeHold::new(reserved?))),
            _ => Err(MediaError::Unexpected { detail: "the executor answered with another result".to_string() }),
        }
    }

    pub async fn image_size(&self, url: String, headers: HashMap<String, String>) -> Result<Option<crate::image_size::ImageSize>, MediaError> {
        match self.outcome(Query::ImageSize { url, headers }).await? {
            Finished::ImageSize(size) => size,
            _ => Err(MediaError::Unexpected { detail: "the executor answered with another result".to_string() }),
        }
    }

    pub fn image_stats(&self) -> Arc<super::image::ImageCounters> { self.image_stats.clone() }

    pub async fn is_cached(&self, source: MediaItem) -> bool {
        matches!(self.outcome(Query::Cached(source)).await, Ok(Finished::Cached(true)))
    }

    pub async fn placeholder(&self, source: MediaItem, ready: bool) -> Placeholder {
        match self.ask(Query::Placeholder(source.clone(), ready)).await {
            Ok((_, Answer::Placeholder(placeholder))) => placeholder,
            _ => placeholder_for(source, ready),
        }
    }

    pub fn open_place(&self, item: MediaItem) -> Result<Arc<MediaPlace>, MediaError> {
        let owner = self.owner();
        self.closing.queue.open(owner, OwnerKind::Place, Declared::PlaceOpen { owner, item })?;
        Ok(Arc::new(MediaPlace::new(self.closing.queue.clone(), owner)))
    }

    pub fn open_whole_declaration(&self) -> Result<Arc<MediaWholeDeclaration>, MediaError> {
        let owner = self.owner();
        self.closing.queue.open(owner, OwnerKind::Declaration, Declared::WholeOpen { owner })?;
        Ok(Arc::new(MediaWholeDeclaration::new(self.closing.queue.clone(), owner)))
    }

    pub fn open_video_window(&self) -> Result<Arc<MediaWindow>, MediaError> {
        self.open_window(WindowKind::Video)
    }

    #[uniffi::method(default(rows_per_page = 1, reversed = false))]
    pub fn open_image_window(&self, name: String, rows_per_page: u32, reversed: bool) -> Result<Arc<MediaWindow>, MediaError> {
        if rows_per_page == 0 {
            return Err(MediaError::Unexpected { detail: "rows_per_page must be positive".to_string() });
        }
        let window = self.open_window(WindowKind::Image(name))?;
        window.lay_out(rows_per_page, reversed);
        Ok(window)
    }

    #[uniffi::method(default(allow_groups = []))]
    pub fn suspend_playback(&self, allow_groups: Vec<String>) -> Result<Arc<PlaybackSuspension>, MediaError> {
        crate::session::check_groups(&allow_groups)?;
        let owner = self.owner();
        self.closing.queue.open(owner, OwnerKind::Suspension, Declared::Suspend { owner, groups: allow_groups })?;
        Ok(Arc::new(PlaybackSuspension::new(self.closing.queue.clone(), owner)))
    }

    pub fn hold_call(&self) -> Result<Arc<CallHold>, MediaError> {
        let owner = self.owner();
        self.closing.queue.open(owner, OwnerKind::Call, Declared::CallHold { owner })?;
        Ok(Arc::new(CallHold::new(self.closing.queue.clone(), owner)))
    }

    pub fn seek(&self, key: String, fraction: f64) -> Result<(), MediaError> {
        self.submit(Declared::KeySeek { key, fraction })
    }

    pub fn set_muted_all(&self, muted: bool) -> Result<(), MediaError> {
        self.submit(Declared::MutedAll(muted))
    }

    pub fn release_all(&self) -> Result<(), MediaError> {
        self.submit(Declared::ReleaseAll)
    }

    pub fn player_events(&self) -> Arc<PlayerEvents> {
        PlayerEvents::new(&self.closing)
    }
}
