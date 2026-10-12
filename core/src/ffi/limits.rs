use crate::limits::{EvictionStrategy, LeadingChunks, Limits as CoreLimits, LimitsError};
use crate::api::PoolSettings as CorePoolSettings;
use crate::memory::DeviceMemory as CoreDeviceMemory;
use crate::pool::ProfileTuning;
use crate::prefetch::PageWindow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Evict {
    Lru,
    Fifo,
    Lfu,
    LargestFirst,
    ExpireAfter { seconds: u32 },
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Limits {
    pub budget: u64,
    pub min_budget: u64,
    pub shrink_to: f64,
    pub memory_budget: u64,
    pub slots: u32,
    pub prefetch_from: u32,
    pub prefetch_to: u32,
    pub leading_chunks: Vec<u32>,
    pub prefetch_budget_mib: u32,
    pub timeout_seconds: u32,
    pub max_concurrent: u32,
    pub dedupe_window_seconds: u32,
    pub dedupe_max_entries: u32,
    pub evict: Evict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct DeviceMemory {
    pub total_mb: u32,
    pub heap_class_mb: Option<u32>,
    pub low_ram: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct PoolSettings {
    pub reach: u32,
    pub hold_off_ms: u64,
    pub trim_delay_ms: u64,
    #[uniffi(default = 2)]
    pub low_ram_slots: u32,
    #[uniffi(default = 0)]
    pub low_ram_reach: u32,
}

impl From<CorePoolSettings> for PoolSettings {
    fn from(core: CorePoolSettings) -> Self {
        PoolSettings {
            reach: core.reach as u32,
            hold_off_ms: core.hold_off_ms,
            trim_delay_ms: core.trim_delay_ms,
            low_ram_slots: core.profile.low_slots as u32,
            low_ram_reach: core.profile.low_reach as u32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ImageWindow {
    pub name: String,
    pub behind: u32,
    pub from: u32,
    pub to: u32,
}

impl From<&ImageWindow> for (String, PageWindow) {
    fn from(window: &ImageWindow) -> Self {
        (window.name.clone(), PageWindow { behind: window.behind as usize, from: window.from as usize, to: window.to as usize })
    }
}

#[uniffi::export]
pub fn default_pool_settings() -> PoolSettings {
    CorePoolSettings::default().into()
}

impl From<PoolSettings> for CorePoolSettings {
    fn from(settings: PoolSettings) -> Self {
        CorePoolSettings {
            reach: settings.reach as usize,
            hold_off_ms: settings.hold_off_ms,
            trim_delay_ms: settings.trim_delay_ms,
            profile: ProfileTuning {
                low_slots: settings.low_ram_slots as usize,
                low_reach: settings.low_ram_reach as usize,
                ..CorePoolSettings::default().profile
            },
        }
    }
}

#[uniffi::export]
pub fn default_limits() -> Limits {
    CoreLimits::default().into()
}

impl From<EvictionStrategy> for Evict {
    fn from(strategy: EvictionStrategy) -> Self {
        match strategy {
            EvictionStrategy::Lru => Evict::Lru,
            EvictionStrategy::Fifo => Evict::Fifo,
            EvictionStrategy::Lfu => Evict::Lfu,
            EvictionStrategy::LargestFirst => Evict::LargestFirst,
            EvictionStrategy::ExpireAfter(seconds) => Evict::ExpireAfter { seconds },
        }
    }
}

impl From<Evict> for EvictionStrategy {
    fn from(evict: Evict) -> Self {
        match evict {
            Evict::Lru => EvictionStrategy::Lru,
            Evict::Fifo => EvictionStrategy::Fifo,
            Evict::Lfu => EvictionStrategy::Lfu,
            Evict::LargestFirst => EvictionStrategy::LargestFirst,
            Evict::ExpireAfter { seconds } => EvictionStrategy::ExpireAfter(seconds),
        }
    }
}

impl From<CoreLimits> for Limits {
    fn from(limits: CoreLimits) -> Self {
        Limits {
            budget: limits.budget,
            min_budget: limits.min_budget,
            shrink_to: limits.shrink_to,
            memory_budget: limits.memory_budget,
            slots: limits.slots,
            prefetch_from: limits.prefetch_from,
            prefetch_to: limits.prefetch_to,
            leading_chunks: limits.leading_chunks.to_vec(),
            prefetch_budget_mib: limits.prefetch_budget_mib,
            timeout_seconds: limits.timeout_seconds,
            max_concurrent: limits.max_concurrent,
            dedupe_window_seconds: limits.dedupe_window_seconds,
            dedupe_max_entries: limits.dedupe_max_entries,
            evict: limits.strategy.into(),
        }
    }
}

impl TryFrom<Limits> for CoreLimits {
    type Error = LimitsError;

    fn try_from(limits: Limits) -> Result<Self, LimitsError> {
        Ok(CoreLimits {
            budget: limits.budget,
            min_budget: limits.min_budget,
            shrink_to: limits.shrink_to,
            memory_budget: limits.memory_budget,
            slots: limits.slots,
            prefetch_from: limits.prefetch_from,
            prefetch_to: limits.prefetch_to,
            leading_chunks: LeadingChunks::new(&limits.leading_chunks)?,
            prefetch_budget_mib: limits.prefetch_budget_mib,
            timeout_seconds: limits.timeout_seconds,
            max_concurrent: limits.max_concurrent,
            dedupe_window_seconds: limits.dedupe_window_seconds,
            dedupe_max_entries: limits.dedupe_max_entries,
            strategy: limits.evict.into(),
        })
    }
}

impl From<DeviceMemory> for CoreDeviceMemory {
    fn from(device: DeviceMemory) -> Self {
        CoreDeviceMemory { total_mb: device.total_mb, heap_class_mb: device.heap_class_mb, low_ram: device.low_ram }
    }
}
