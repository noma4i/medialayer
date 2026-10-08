use crate::cache;
use crate::pool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, uniffi::Enum)]
pub enum Kind {
    Image,
    Video,
    Audio,
}

impl From<cache::Kind> for Kind {
    fn from(kind: cache::Kind) -> Self {
        match kind {
            cache::Kind::Image => Kind::Image,
            cache::Kind::Stream => Kind::Video,
            cache::Kind::Audio => Kind::Audio,
        }
    }
}

impl From<Kind> for cache::Kind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Image => cache::Kind::Image,
            Kind::Video => cache::Kind::Stream,
            Kind::Audio => cache::Kind::Audio,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Pressure {
    Moderate,
    Critical,
}

impl From<Pressure> for pool::Pressure {
    fn from(pressure: Pressure) -> Self {
        match pressure {
            Pressure::Moderate => pool::Pressure::Moderate,
            Pressure::Critical => pool::Pressure::Critical,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum MediaState {
    Idle,
    Loading,
    Ready,
    Failed { detail: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct KindUsage {
    pub kind: Kind,
    pub bytes: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct SlotCounts {
    pub hot: u32,
    pub warm: u32,
    pub cold: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ReleaseCounts {
    pub timer: u64,
    pub pressure: u64,
    pub background: u64,
    pub source_change: u64,
    pub explicit: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MediaStats {
    pub total_bytes: u64,
    pub file_count: u64,
    pub kinds: Vec<KindUsage>,
    pub slots: SlotCounts,
    pub released: ReleaseCounts,
    pub memory_bytes: u64,
    pub evictions: u64,
    pub max_hold_ms: u64,
    pub diagnostics: Vec<String>,
}

impl MediaStats {
    pub fn collect(cache: &cache::Stats, pool: &pool::Stats, diagnostics: Vec<String>) -> MediaStats {
        let released = pool.released;
        MediaStats {
            total_bytes: cache.total_bytes,
            file_count: cache.file_count,
            kinds: cache.per_kind.iter().map(|(kind, usage)| KindUsage { kind: (*kind).into(), bytes: usage.bytes, files: usage.files }).collect(),
            slots: SlotCounts { hot: pool.hot as u32, warm: pool.warm as u32, cold: pool.cold as u32 },
            released: ReleaseCounts {
                timer: released.timer,
                pressure: released.pressure,
                background: released.background,
                source_change: released.source_change,
                explicit: released.explicit,
            },
            memory_bytes: pool.memory_bytes,
            evictions: cache.evictions,
            max_hold_ms: cache.max_hold.as_millis() as u64,
            diagnostics,
        }
    }
}
