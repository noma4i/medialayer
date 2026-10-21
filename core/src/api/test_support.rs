use std::path::PathBuf;
use std::time::Duration;

const WAIT_LIMIT: Duration = Duration::from_secs(20);

pub async fn eventually_async<F: std::future::Future<Output = bool>>(mut condition: impl FnMut() -> F) {
    let deadline = std::time::Instant::now() + WAIT_LIMIT;
    while std::time::Instant::now() < deadline {
        if condition().await {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("condition never became true within {WAIT_LIMIT:?}");
}

pub async fn eventually(mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + WAIT_LIMIT;
    while std::time::Instant::now() < deadline {
        if condition() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("condition never became true within {WAIT_LIMIT:?}");
}

use super::assemble::{Assembly, PoolSettings};
use super::ports::{CacheControl, MemoryRelease, PoolControl, PrefetchControl, SettingsStore, SlotDirectives, TransportControl};
use super::slots::{PlayerSlot, PlayerSlotFactory, SlotCall, SlotSample};
use super::source::{MediaItem, PlaybackRoute};
use super::types::Kind;
use crate::cache::{self, classify_kind, Cache, CacheStore};
use crate::fetch::mock::MockFetcher;
use crate::limits::{Limits, LimitsError};
use crate::memory::{DeviceMemory, Step};
use crate::pool::{Command, Directive, Pressure, Released, PoolStats};
use crate::proxy::{ByteStore, Config as ProxyConfig, Proxy};
use crate::session::WatchdogConfig;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Default)]
pub struct LoadGate {
    closed: Mutex<bool>,
    opened: Condvar,
    entered: AtomicUsize,
}

impl LoadGate {
    pub fn close(&self) {
        *self.closed.lock().unwrap() = true;
    }

    pub fn open(&self) {
        *self.closed.lock().unwrap() = false;
        self.opened.notify_all();
    }

    pub fn entered(&self) -> usize {
        self.entered.load(Ordering::SeqCst)
    }

    fn pass(&self) {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let closed = self.closed.lock().unwrap();
        let _ = self.opened.wait_timeout_while(closed, Duration::from_secs(10), |closed| *closed).unwrap();
    }
}

#[derive(Default)]
pub struct FakeCache {
    pub stats: Mutex<cache::CacheStats>,
    pub applied: Mutex<Vec<Limits>>,
    pub cleared: Mutex<Vec<Option<cache::Kind>>>,
    pub purged: Mutex<Vec<String>>,
    pub evictions: AtomicUsize,
    pub cached: Mutex<Vec<String>>,
}

#[async_trait]
impl CacheControl for FakeCache {
    async fn stats(&self) -> cache::CacheStats {
        self.stats.lock().unwrap().clone()
    }

    async fn set_limits(&self, limits: Limits) -> Result<(), LimitsError> {
        self.applied.lock().unwrap().push(limits);
        Ok(())
    }

    async fn clear(&self, kind: Option<cache::Kind>) {
        self.cleared.lock().unwrap().push(kind);
    }

    async fn purge(&self, url: &str) {
        self.purged.lock().unwrap().push(url.to_string());
    }

    async fn evict_now(&self) {
        self.evictions.fetch_add(1, Ordering::SeqCst);
    }

    async fn contains(&self, key: &str) -> bool {
        self.cached.lock().unwrap().iter().any(|cached| cached == key)
    }
}

#[derive(Default)]
pub struct FakeSettings {
    pub values: Mutex<HashMap<String, String>>,
    pub reads: AtomicUsize,
    pub writes: Mutex<Vec<(String, String)>>,
    pub removed: Mutex<Vec<String>>,
}

impl SettingsStore for FakeSettings {
    fn read(&self, key: String) -> Option<String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.values.lock().unwrap().get(&key).cloned()
    }

    fn write(&self, key: String, value: String) {
        self.writes.lock().unwrap().push((key.clone(), value.clone()));
        self.values.lock().unwrap().insert(key, value);
    }

    fn remove(&self, key: String) {
        self.removed.lock().unwrap().push(key.clone());
        self.values.lock().unwrap().remove(&key);
    }
}

pub fn pool_stats() -> PoolStats {
    PoolStats { hot: 1, warm: 2, cold: 3, reuses: 0, released: Released::default(), memory_bytes: 0 }
}

#[derive(Default)]
pub struct FakePool {
    pub stats: Mutex<Option<PoolStats>>,
    pub events: Mutex<Vec<String>>,
    pub steps: Mutex<Vec<Step>>,
    pub slots: Mutex<Vec<usize>>,
    pub to_return: Mutex<Vec<Directive>>,
}

impl PoolControl for FakePool {
    fn stats(&self) -> PoolStats {
        self.stats.lock().unwrap().unwrap_or_else(pool_stats)
    }

    fn set_step(&self, step: Step) {
        self.steps.lock().unwrap().push(step);
    }

    fn release_under(&self, _now_ms: u64, pressure: Pressure) -> Vec<Directive> {
        self.events.lock().unwrap().push(format!("release_under {pressure:?}"));
        self.to_return.lock().unwrap().clone()
    }

    fn background(&self) -> Vec<Directive> {
        self.events.lock().unwrap().push("background".to_string());
        self.to_return.lock().unwrap().clone()
    }

    fn foreground(&self) {
        self.events.lock().unwrap().push("foreground".to_string());
    }

    fn set_slots(&self, slots: usize) {
        self.slots.lock().unwrap().push(slots);
    }
}

#[derive(Default)]
pub struct FakePrefetch {
    pub stops: AtomicUsize,
    pub forgotten: AtomicUsize,
    pub limits: Mutex<Vec<Limits>>,
}

impl PrefetchControl for FakePrefetch {
    fn forget_loaded(&self) {
        self.forgotten.fetch_add(1, Ordering::SeqCst);
    }

    fn stop(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }

    fn set_limits(&self, limits: &Limits) {
        self.limits.lock().unwrap().push(*limits);
    }
}

#[derive(Default)]
pub struct FakeTransport {
    pub limits: Mutex<Vec<Limits>>,
}

impl TransportControl for FakeTransport {
    fn set_limits(&self, limits: &Limits) {
        self.limits.lock().unwrap().push(*limits);
    }
}

#[derive(Default)]
pub struct FakeRelease {
    pub drops: AtomicUsize,
}

impl MemoryRelease for FakeRelease {
    fn drop_caches(&self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
pub struct FakeDirectives {
    pub applied: Mutex<Vec<Vec<Directive>>>,
}

impl SlotDirectives for FakeDirectives {
    fn apply(&self, directives: Vec<Directive>) {
        self.applied.lock().unwrap().push(directives);
    }
}

pub fn unload(slot: usize) -> Directive {
    Directive::Slot(Command::Unload { slot })
}

static SLOW_ASSEMBLIES: Mutex<Vec<(PathBuf, Duration)>> = Mutex::new(Vec::new());

pub fn slow_assembly(dir: &Path, delay: Duration) {
    SLOW_ASSEMBLIES.lock().unwrap().push((dir.to_path_buf(), delay));
}

pub fn assembly_delay(dir: &Path) -> Option<Duration> {
    SLOW_ASSEMBLIES.lock().unwrap().iter().find(|(slow, _)| slow == dir).map(|(_, delay)| *delay)
}

pub fn shared<T: Default>() -> Arc<T> {
    Arc::new(T::default())
}

pub struct ProxyRig {
    pub _dir: tempfile::TempDir,
    pub cache: Cache,
    pub proxy: Proxy,
    pub fetcher: Arc<MockFetcher>,
}

pub async fn proxy_rig() -> ProxyRig {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), Limits::default(), u64::MAX);
    cache.ready().await;
    let store: Arc<dyn ByteStore> = Arc::new(CacheStore::new(cache.clone(), Arc::new(classify_kind)));
    let fetcher = Arc::new(MockFetcher::new());
    let proxy = Proxy::start(ProxyConfig::default(), fetcher.clone(), store).await.unwrap();
    ProxyRig { _dir: dir, cache, proxy, fetcher }
}

pub struct MockSlot {
    pub calls: Mutex<Vec<SlotCall>>,
    pub gate: Arc<LoadGate>,
    pub sample: Mutex<SlotSample>,
    pub samples: AtomicUsize,
    pub advance_ms: u64,
    pub seeked: tokio::sync::Notify,
}

impl MockSlot {
    pub fn calls(&self) -> Vec<SlotCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn loads(&self) -> usize {
        self.calls().iter().filter(|call| matches!(call, SlotCall::Load { .. })).count()
    }
}

impl PlayerSlot for MockSlot {
    fn load(&self, key: String, route: PlaybackRoute) {
        self.gate.pass();
        self.calls.lock().unwrap().push(SlotCall::Load { key, route });
    }

    fn unload(&self) {
        self.calls.lock().unwrap().push(SlotCall::Unload);
    }

    fn preroll(&self) {
        self.calls.lock().unwrap().push(SlotCall::Preroll);
    }

    fn play(&self) {
        self.calls.lock().unwrap().push(SlotCall::Play);
    }

    fn pause(&self) {
        self.calls.lock().unwrap().push(SlotCall::Pause);
    }

    fn mute(&self) {
        self.calls.lock().unwrap().push(SlotCall::Mute);
    }

    fn unmute(&self) {
        self.calls.lock().unwrap().push(SlotCall::Unmute);
    }

    fn attach(&self, view_token: u64) {
        self.calls.lock().unwrap().push(SlotCall::Attach { view_token });
    }

    fn detach(&self) {
        self.calls.lock().unwrap().push(SlotCall::Detach);
    }

    fn seek(&self, position_ms: u64) {
        self.calls.lock().unwrap().push(SlotCall::Seek { position_ms });
        self.seeked.notify_one();
    }

    fn set_rate(&self, rate: f32) {
        self.calls.lock().unwrap().push(SlotCall::Rate { rate });
    }

    fn set_readahead(&self, ms: u64) {
        self.calls.lock().unwrap().push(SlotCall::Readahead { ms });
    }

    fn sample(&self) -> SlotSample {
        let taken = self.samples.fetch_add(1, Ordering::SeqCst) as u64 + 1;
        let mut sample = *self.sample.lock().unwrap();
        sample.position_ms += self.advance_ms * taken;
        sample
    }
}

#[derive(Default)]
pub struct MockFactory {
    pub slots: Mutex<HashMap<u32, Arc<MockSlot>>>,
    pub created: Mutex<Vec<u32>>,
    pub gate: Arc<LoadGate>,
    pub advance_ms: Mutex<u64>,
}

impl MockFactory {
    pub fn slot(&self, index: u32) -> Arc<MockSlot> {
        self.slots.lock().unwrap().get(&index).cloned().unwrap_or_else(|| panic!("slot {index} was never created"))
    }

    pub fn slot_holding(&self, key: &str) -> Option<(u32, Arc<MockSlot>)> {
        let slots = self.slots.lock().unwrap();
        slots
            .iter()
            .find(|(_, slot)| slot.calls().iter().rev().find_map(|call| match call {
                SlotCall::Load { key: loaded, .. } => Some(loaded == key),
                SlotCall::Unload => Some(false),
                _ => None,
            }).unwrap_or(false))
            .map(|(index, slot)| (*index, slot.clone()))
    }
}

impl PlayerSlotFactory for MockFactory {
    fn create(&self, slot: u32) -> Arc<dyn PlayerSlot> {
        let idle = SlotSample { position_ms: 0, duration_ms: Some(60_000), is_looping: false, view_attached: false, frame_rendered: false };
        let player = Arc::new(MockSlot {
            calls: Mutex::default(),
            gate: self.gate.clone(),
            sample: Mutex::new(idle),
            samples: AtomicUsize::new(0),
            advance_ms: *self.advance_ms.lock().unwrap(),
            seeked: tokio::sync::Notify::new(),
        });
        self.created.lock().unwrap().push(slot);
        self.slots.lock().unwrap().insert(slot, player.clone());
        player
    }
}

pub fn fast_watchdog() -> WatchdogConfig {
    WatchdogConfig {
        first_window_ms: 40,
        progress_window_ms: 40,
        cooldown_ms: 120,
        startup_delay_ms: 15,
        retry_delays_ms: vec![15; 5],
        foreground_delays_ms: vec![15; 3],
        ..WatchdogConfig::default()
    }
}

pub fn assembly(dir: &Path, fetcher: Arc<MockFetcher>, factory: Arc<MockFactory>) -> Assembly {
    Assembly {
        available_bytes: u64::MAX,
        dir: dir.to_path_buf(),
        fetcher,
        settings: shared::<FakeSettings>(),
        factory: Some(factory),
        device: DeviceMemory { total_mb: 8192, heap_class_mb: None, low_ram: false },
        defaults: Limits::default(),
        pool: PoolSettings::default(),
        namespace_headers: Vec::new(),
        image_windows: Vec::new(),
        forward_headers: None,
        eviction_overrides: Default::default(),
        silent: false,
        watchdog: fast_watchdog(),
    }
}

pub fn video(url: &str) -> MediaItem {
    MediaItem::bare(url, Kind::Video)
}

pub fn videos(count: usize) -> Vec<Option<MediaItem>> {
    (0..count).map(|index| Some(video(&format!("https://o.test/v{index}.mp4")))).collect()
}

pub fn url_of(index: usize) -> String {
    format!("https://o.test/v{index}.mp4")
}

pub struct Rig {
    pub _dir: tempfile::TempDir,
    pub parts: super::assemble::Parts,
    pub factory: Arc<MockFactory>,
    pub fetcher: Arc<MockFetcher>,
}

impl Rig {
    pub fn holder(&self, url: &str) -> (u32, Arc<MockSlot>) {
        self.factory.slot_holding(url).unwrap_or_else(|| panic!("no slot holds {url}"))
    }
}

pub async fn rig() -> Rig {
    rig_with(|_| {}).await
}

pub async fn rig_with(adjust: impl FnOnce(&mut Assembly)) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(MockFetcher::new());
    let factory: Arc<MockFactory> = Arc::default();
    let mut config = assembly(dir.path(), fetcher.clone(), factory.clone());
    adjust(&mut config);
    let parts = super::assemble::assemble(config).await.unwrap();
    parts.cache.ready().await;
    Rig { _dir: dir, parts, factory, fetcher }
}
