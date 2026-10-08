use super::error::MediaError;
use super::places::Places;
use super::ports::{CacheControl, PrefetchControl, SettingsStore, TransportControl};
use super::routes::Routes;
use super::slots::{PlayerSlotFactory, SlotRunner};
use super::stage::{Stage, StageParts};
use crate::cache::{classify_kind, Cache, CacheStore, Clock, SystemClock};
use crate::fetch::Fetcher;
use crate::identity::cache_key;
use crate::limits::{Limits, LimitsError};
use crate::memory::{effective_ceiling, DeviceMemory, Ledger, ShareTuning};
use crate::pool::{DeviceProfile, PoolConfig, ProfileTuning, Ring};
use crate::prefetch::{PrefetchLedger, Prefetcher, ProxyLoader};
use crate::proxy::{ByteStore, Config as ProxyConfig, Proxy};
use crate::session::WatchdogConfig;
use crate::spec::PoolSpec;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Handle;

const MIN_TICK_MS: u64 = 50;
const MAX_TICK_MS: u64 = 5_000;
const TICKS_PER_TRIM_DELAY: u64 = 4;
const LOW_RAM_BELOW_MB: u32 = 3_072;
const LOW_RAM_SLOTS: usize = 2;
const LOW_RAM_REACH: usize = 0;
const LOW_RAM_BUFFER_SECONDS: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolSettings {
    pub reach: usize,
    pub hold_off_ms: u64,
    pub trim_delay_ms: u64,
    pub profile: ProfileTuning,
}

impl From<&PoolSpec> for PoolSettings {
    fn from(spec: &PoolSpec) -> Self {
        PoolSettings {
            reach: spec.reach as usize,
            hold_off_ms: u64::from(spec.hold_off) * 1_000,
            trim_delay_ms: u64::from(spec.trim_delay) * 1_000,
            profile: ProfileTuning {
                low_ram_below_mb: LOW_RAM_BELOW_MB,
                low_slots: spec.low_ram_slots.map_or(LOW_RAM_SLOTS, |slots| slots as usize),
                low_reach: spec.low_ram_reach.map_or(LOW_RAM_REACH, |reach| reach as usize),
                low_buffer_seconds: LOW_RAM_BUFFER_SECONDS,
            },
        }
    }
}

impl Default for PoolSettings {
    fn default() -> Self {
        PoolSettings::from(&PoolSpec::default())
    }
}

pub struct Assembly {
    pub dir: PathBuf,
    pub fetcher: Arc<dyn Fetcher>,
    pub settings: Arc<dyn SettingsStore>,
    pub factory: Option<Arc<dyn PlayerSlotFactory>>,
    pub device: DeviceMemory,
    pub defaults: Limits,
    pub pool: PoolSettings,
    pub namespace_headers: Vec<String>,
    pub watchdog: WatchdogConfig,
    pub stats_interval: Duration,
}

pub struct Parts {
    pub cache: Cache,
    pub proxy: Proxy,
    pub prefetcher: Prefetcher,
    pub stage: Stage,
    pub places: Arc<Places>,
    pub routes: Arc<Routes>,
    pub settings: Arc<dyn SettingsStore>,
    pub ledger: Ledger,
    pub clock: Arc<dyn Clock>,
    pub defaults: Limits,
    pub device: DeviceMemory,
    pub stats_interval: Duration,
    pub pool: PoolSettings,
}

pub fn ledger_for(limits: &Limits, device: DeviceMemory) -> Ledger {
    Ledger::new(effective_ceiling(limits.memory_budget, device, ShareTuning::default()))
}

pub fn pool_config(limits: &Limits, device: DeviceMemory, settings: &PoolSettings) -> PoolConfig {
    let profile = DeviceProfile { ram_mb: device.total_mb, low_ram: device.low_ram };
    PoolConfig {
        hold_off_ms: settings.hold_off_ms,
        trim_delay_ms: settings.trim_delay_ms,
        ..PoolConfig::from_limits(limits, settings.reach).for_profile(profile, settings.profile)
    }
}

#[async_trait]
impl CacheControl for Cache {
    async fn stats(&self) -> crate::cache::Stats {
        Cache::stats(self).await
    }

    async fn set_limits(&self, limits: Limits) -> Result<(), LimitsError> {
        Cache::set_limits(self, limits).await
    }

    async fn clear(&self, kind: Option<crate::cache::Kind>) {
        Cache::clear(self, kind).await;
    }

    async fn purge(&self, url: &str) {
        let key = cache_key(url);
        Cache::remove(self, &key).await;
        Cache::purge_prefix(self, &format!("{key}#")).await;
    }

    async fn evict_now(&self) {
        Cache::evict_now(self).await;
    }

    async fn contains(&self, key: &str) -> bool {
        Cache::contains(self, key).await
    }
}

impl TransportControl for Proxy {
    fn set_limits(&self, limits: &Limits) {
        Proxy::set_limits(self, limits);
    }
}

impl PrefetchControl for Prefetcher {
    fn stop(&self) {
        self.stop_all();
    }

    fn set_limits(&self, limits: &Limits) {
        Prefetcher::set_limits(self, limits);
    }
}

pub async fn assemble(config: Assembly) -> Result<Parts, MediaError> {
    let Assembly { dir, fetcher, settings, factory, device, defaults, pool, namespace_headers, watchdog, stats_interval } = config;
    defaults.validate()?;
    let handle = Handle::current();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let ledger = ledger_for(&defaults, device);
    let cache = Cache::open(dir, defaults);
    let store: Arc<dyn ByteStore> = Arc::new(CacheStore::new(cache.clone(), Arc::new(classify_kind)));
    let proxy = Proxy::start(ProxyConfig { ledger: ledger.clone(), namespace_headers, ..ProxyConfig::from_limits(&defaults) }, fetcher, store)
        .await
        .map_err(|error| MediaError::Unexpected { detail: error.to_string() })?;
    let dedupe_window = Duration::from_secs(defaults.dedupe_window_seconds.into());
    let prefetch_ledger = PrefetchLedger::new(dedupe_window, defaults.dedupe_max_entries as usize);
    let prefetcher = Prefetcher::new(Arc::new(ProxyLoader::new(proxy.clone())), &defaults, prefetch_ledger, clock.clone());
    let routes = Arc::new(Routes::new(proxy.clone(), Arc::new(cache.clone())));
    let places = Arc::new(Places::new(handle.clone()));
    let ring_config = pool_config(&defaults, device, &pool);
    let tick = Duration::from_millis((ring_config.trim_delay_ms / TICKS_PER_TRIM_DELAY).clamp(MIN_TICK_MS, MAX_TICK_MS));
    let stage = Stage::new(
        handle.clone(),
        StageParts {
            ring: Ring::with_ledger(ring_config, ledger.clone()),
            runner: Arc::new(SlotRunner::new(handle, factory)),
            routes: routes.clone(),
            places: places.clone(),
            prefetcher: prefetcher.clone(),
            clock: clock.clone(),
            watchdog,
            tick,
        },
    );
    Ok(Parts { cache, proxy, prefetcher, stage, places, routes, settings, ledger, clock, defaults, device, stats_interval, pool })
}
