use std::path::PathBuf;
use std::time::Duration;

const WAIT_LIMIT: Duration = Duration::from_secs(20);

#[track_caller]
pub fn eventually_async<F: std::future::Future<Output = bool>>(mut condition: impl FnMut() -> F) -> impl std::future::Future<Output = ()> {
    let caller = std::panic::Location::caller();
    async move { tokio::time::timeout(WAIT_LIMIT, async {
        loop {
            let seen = crate::test_signal::current();
            if condition().await { return; }
            crate::test_signal::changed_since(seen).await;
        }
    }).await.unwrap_or_else(|_| panic!("condition at {caller} never became true")); }
}

#[track_caller]
pub fn eventually(condition: impl FnMut() -> bool) -> impl std::future::Future<Output = ()> {
    wait_until(condition, WAIT_LIMIT)
}

#[track_caller]
pub fn wait_until(mut condition: impl FnMut() -> bool, limit: Duration) -> impl std::future::Future<Output = ()> {
    let caller = std::panic::Location::caller();
    async move { tokio::time::timeout(limit, async {
        loop {
            let seen = crate::test_signal::current();
            if condition() { return; }
            crate::test_signal::changed_since(seen).await;
        }
    }).await.unwrap_or_else(|_| panic!("condition at {caller} never became true")); }
}

use super::assemble::{Assembly, PoolSettings};
use super::ports::{CacheControl, MemoryRelease, PoolControl, PrefetchControl, SettingsStore, SlotDirectives, TransportControl};
use super::slots::{CreatedPlayer, PlayerSlot, PlayerSlotFactory, SlotCall, SlotSample};
use super::source::MediaItem;
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
use std::sync::{Arc, Mutex};

pub struct LoadGate {
    closed: tokio::sync::watch::Sender<bool>,
    entered: AtomicUsize,
}

impl Default for LoadGate {
    fn default() -> LoadGate {
        LoadGate { closed: tokio::sync::watch::channel(false).0, entered: AtomicUsize::new(0) }
    }
}

pub struct HeldGate(Arc<LoadGate>);

impl Drop for HeldGate {
    fn drop(&mut self) {
        self.0.open();
    }
}

impl LoadGate {
    pub fn close(&self) {
        self.closed.send_replace(true);
    }

    pub fn open(&self) {
        self.closed.send_replace(false);
    }

    pub fn hold(self: &Arc<Self>) -> HeldGate {
        self.close();
        HeldGate(self.clone())
    }

    pub fn entered(&self) -> usize {
        self.entered.load(Ordering::SeqCst)
    }

    async fn pass(&self) {
        self.entered.fetch_add(1, Ordering::SeqCst);
        crate::test_signal::fire();
        let mut closed = self.closed.subscribe();
        let _ = tokio::time::timeout(Duration::from_secs(10), closed.wait_for(|closed| !*closed)).await;
    }
}

#[derive(Default)]
pub struct FakeCache {
    pub before_delete: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
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
        if let Some(check) = &*self.before_delete.lock().unwrap() { check(); }
        self.cleared.lock().unwrap().push(kind);
    }

    async fn purge(&self, url: &str) {
        if let Some(check) = &*self.before_delete.lock().unwrap() { check(); }
        self.purged.lock().unwrap().push(url.to_string());
    }

    async fn evict_now(&self) {
        self.evictions.fetch_add(1, Ordering::SeqCst);
    }

    async fn contains(&self, key: &str) -> bool {
        self.cached.lock().unwrap().iter().any(|cached| cached == key)
    }

    fn image_size(&self, _: &str) -> Option<crate::image_size::ImageSize> { None }

    fn set_image_size(&self, _: &str, _: crate::image_size::ImageSize) -> bool { false }
}

#[async_trait]
impl super::tones::ToneStore for FakeCache {
    fn key_for(&self, url: &str, _: &HashMap<String, String>) -> String { url.to_owned() }
    async fn tone(&self, _: &str) -> Option<crate::analysis::ToneData> { None }
    async fn set_tone(&self, _: &str, _: crate::analysis::ToneData) -> bool { false }
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
    PoolStats { hot: 1, playing: 1, warm: 2, cold: 3, reuses: 0, released: Released::default(), memory_bytes: 0 }
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
        crate::test_signal::fire();
    }

    fn release_under(&self, _now_ms: u64, pressure: Pressure) -> Vec<Directive> {
        self.events.lock().unwrap().push(format!("release_under {pressure:?}"));
        crate::test_signal::fire();
        self.to_return.lock().unwrap().clone()
    }

    fn background(&self) -> Vec<Directive> {
        self.events.lock().unwrap().push("background".to_string());
        crate::test_signal::fire();
        self.to_return.lock().unwrap().clone()
    }

    fn foreground(&self) {
        self.events.lock().unwrap().push("foreground".to_string());
        crate::test_signal::fire();
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
    fn forget_loaded(&self, _url: Option<&str>) {
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
    pub forgotten_downloads: Mutex<Vec<Option<String>>>,
}

impl TransportControl for FakeTransport {
    fn forget_downloads(&self, url: Option<&str>) {
        self.forgotten_downloads.lock().unwrap().push(url.map(str::to_string));
    }
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
        crate::test_signal::fire();
    }
}

#[derive(Default)]
pub struct FakeDirectives {
    pub applied: Mutex<Vec<Vec<Directive>>>,
}

impl SlotDirectives for FakeDirectives {
    fn apply(&self, directives: Vec<Directive>) {
        self.applied.lock().unwrap().push(directives);
        crate::test_signal::fire();
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
    let cache = Cache::open(dir.path(), Limits::default());
    cache.ready().await;
    let store: Arc<dyn ByteStore> = Arc::new(CacheStore::new(cache.clone(), Arc::new(classify_kind)));
    let fetcher = Arc::new(MockFetcher::new());
    let proxy = Proxy::start(ProxyConfig::default(), fetcher.clone(), store).await.unwrap();
    ProxyRig { _dir: dir, cache, proxy, fetcher }
}

pub struct MockSlot {
    pub calls: Mutex<Vec<SlotCall>>,
    pub gate: Arc<LoadGate>,
    pub seek_gate: Arc<LoadGate>,
    pub unload_gate: Arc<LoadGate>,
    pub dispose_gate: Arc<LoadGate>,
    pub live: AtomicUsize,
    pub disposed: AtomicUsize,
    pub disposals: Arc<AtomicUsize>,
    pub sample: Mutex<SlotSample>,
    pub samples: AtomicUsize,
    pub on_sample: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    pub advance_ms: u64,
    pub seeked: tokio::sync::Notify,
    pub started: AtomicUsize,
    pub in_flight: AtomicUsize,
    pub peak: AtomicUsize,
    pub cancelled: AtomicUsize,
}

struct InFlight<'a>(&'a MockSlot, bool);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
        if !self.1 {
            self.0.cancelled.fetch_add(1, Ordering::SeqCst);
        }
        crate::test_signal::fire();
    }
}

impl MockSlot {
    pub fn calls(&self) -> Vec<SlotCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn loads(&self) -> usize {
        self.calls().iter().filter(|call| matches!(call, SlotCall::Load { .. })).count()
    }

    pub fn applies_started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }

    pub fn peak_applies(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    pub fn cancelled_applies(&self) -> usize {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn generation(&self) -> u64 {
        self.calls().iter().rev().find_map(|call| if let SlotCall::Load { generation, .. } = call { Some(*generation) } else { None }).unwrap_or(0)
    }
}

#[async_trait]
impl PlayerSlot for MockSlot {
    async fn apply(&self, command: SlotCall) {
        self.started.fetch_add(1, Ordering::SeqCst);
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        crate::test_signal::fire();
        let mut flight = InFlight(self, false);
        if matches!(command, SlotCall::Load { .. }) {
            self.gate.pass().await;
        }
        if matches!(command, SlotCall::Seek { .. }) {
            self.seek_gate.pass().await;
        }
        if matches!(command, SlotCall::Unload) {
            self.unload_gate.pass().await;
        }
        match command {
            SlotCall::Load { .. } => {
                self.live.fetch_add(1, Ordering::SeqCst);
            }
            SlotCall::Unload => {
                self.live.fetch_sub(1, Ordering::SeqCst);
            }
            _ => {}
        }
        flight.1 = true;
        let is_seek = matches!(command, SlotCall::Seek { .. });
        self.calls.lock().unwrap().push(command);
        crate::test_signal::fire();
        if is_seek {
            self.seeked.notify_one();
        }
    }

    async fn dispose(&self) {
        self.dispose_gate.pass().await;
        self.live.store(0, Ordering::SeqCst);
        self.disposed.fetch_add(1, Ordering::SeqCst);
        self.disposals.fetch_add(1, Ordering::SeqCst);
        crate::test_signal::fire();
    }

    fn sample(&self) -> SlotSample {
        let taken = self.samples.fetch_add(1, Ordering::SeqCst) as u64 + 1;
        crate::test_signal::fire();
        let hook = self.on_sample.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
        }
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
    pub seek_gate: Arc<LoadGate>,
    pub unload_gate: Arc<LoadGate>,
    pub dispose_gate: Arc<LoadGate>,
    pub create_gate: Arc<LoadGate>,
    pub disposals: Arc<AtomicUsize>,
    pub advance_ms: Mutex<u64>,
}

impl MockFactory {
    pub fn slot(&self, index: u32) -> Arc<MockSlot> {
        self.slots.lock().unwrap().get(&index).cloned().unwrap_or_else(|| panic!("slot {index} was never created"))
    }

    pub fn live_sources(&self) -> usize {
        self.slots.lock().unwrap().values().map(|slot| slot.live.load(Ordering::SeqCst)).sum()
    }

    pub fn disposed_slots(&self) -> usize {
        self.disposals.load(Ordering::SeqCst)
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

#[async_trait]
impl PlayerSlotFactory for MockFactory {
    async fn create(&self, slot: u32) -> CreatedPlayer {
        self.create_gate.pass().await;
        let idle = SlotSample { position_ms: 0, duration_ms: Some(60_000), is_looping: false, view_attached: false, frame_rendered: false };
        let player = Arc::new(MockSlot {
            calls: Mutex::default(),
            gate: self.gate.clone(),
            seek_gate: self.seek_gate.clone(),
            unload_gate: self.unload_gate.clone(),
            dispose_gate: self.dispose_gate.clone(),
            live: AtomicUsize::new(0),
            disposed: AtomicUsize::new(0),
            disposals: self.disposals.clone(),
            sample: Mutex::new(idle),
            samples: AtomicUsize::new(0),
            on_sample: Mutex::new(None),
            advance_ms: *self.advance_ms.lock().unwrap(),
            seeked: tokio::sync::Notify::new(),
            started: AtomicUsize::new(0),
            in_flight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            cancelled: AtomicUsize::new(0),
        });
        self.created.lock().unwrap().push(slot);
        self.slots.lock().unwrap().insert(slot, player.clone());
        crate::test_signal::fire();
        CreatedPlayer { player }
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
        failures: Arc::default(),
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

pub struct TokioClock(tokio::time::Instant);

impl TokioClock {
    pub fn new() -> TokioClock { TokioClock(tokio::time::Instant::now()) }
}

impl crate::cache::Clock for TokioClock {
    fn now_ms(&self) -> u64 { self.0.elapsed().as_millis() as u64 }
}

pub fn replace_stage_clock(rig: &mut Rig, pool: PoolSettings, clock: Arc<dyn crate::cache::Clock>) {
    let parts = &mut rig.parts;
    let ring = crate::pool::Ring::with_ledger(super::assemble::pool_config(&parts.limits, parts.device, &pool), parts.ledger.clone());
    let runner = Arc::new(super::slots::SlotRunner::new(tokio::runtime::Handle::current(), Some(rig.factory.clone())));
    parts.stage = super::stage::Stage::new(tokio::runtime::Handle::current(), super::stage::StageParts {
        ring, runner, routes: parts.routes.clone(), places: parts.places.clone(), prefetcher: parts.prefetcher.clone(), clock,
        watchdog: fast_watchdog(), silent: false, change: parts.change.clone(), ledger: parts.ledger.clone(),
    });
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
