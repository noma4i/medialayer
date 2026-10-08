use serde::{Deserialize, Serialize};

use crate::limits::{EvictionStrategy, Limits};

const DEFAULT_FORWARD_HEADERS: [&str; 8] =
    ["Authorization", "Cookie", "User-Agent", "Connection", "Accept", "Accept-Encoding", "Accept-Language", "Range"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Evict {
    Lru,
    Fifo,
    Lfu,
    LargestFirst,
    ExpireAfter { seconds: u32 },
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CacheSpec {
    pub budget: u64,
    pub min: u64,
    pub shrink_to: u32,
    pub evict: Evict,
    pub memory: u64,
    pub timeout: u32,
    pub max_concurrent: u32,
    pub dedupe_window: u32,
    pub dedupe_entries: u32,
    pub namespace_headers: Vec<String>,
}

impl Default for CacheSpec {
    fn default() -> Self {
        let limits = Limits::default();
        CacheSpec {
            budget: limits.budget,
            min: limits.min_budget,
            shrink_to: 90,
            evict: Evict::Lru,
            memory: limits.memory_budget,
            timeout: limits.timeout_seconds,
            max_concurrent: limits.max_concurrent,
            dedupe_window: limits.dedupe_window_seconds,
            dedupe_entries: limits.dedupe_max_entries,
            namespace_headers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PrefetchSpec {
    pub from: u32,
    pub to: u32,
    pub depth: u32,
}

impl Default for PrefetchSpec {
    fn default() -> Self {
        let limits = Limits::default();
        PrefetchSpec { from: limits.prefetch_from, to: limits.prefetch_to, depth: limits.depth_seconds }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PoolSpec {
    pub slots: u32,
    pub reach: u32,
    pub hold_off: u32,
    pub trim_delay: u32,
}

impl Default for PoolSpec {
    fn default() -> Self {
        PoolSpec { slots: Limits::default().slots, reach: 1, hold_off: 1, trim_delay: 30 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoSpec {
    pub name: String,
    #[serde(default)]
    pub cover_field: Option<String>,
    #[serde(default)]
    pub prefetch: PrefetchSpec,
    #[serde(default)]
    pub pool: PoolSpec,
    #[serde(default = "default_forward_headers")]
    pub forward_headers: Vec<String>,
    #[serde(default)]
    pub evict: Option<Evict>,
}

fn default_forward_headers() -> Vec<String> {
    DEFAULT_FORWARD_HEADERS.iter().map(|header| header.to_string()).collect()
}

impl VideoSpec {
    pub fn new(name: &str) -> Self {
        VideoSpec {
            name: name.to_string(),
            cover_field: None,
            prefetch: PrefetchSpec::default(),
            pool: PoolSpec::default(),
            forward_headers: default_forward_headers(),
            evict: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageSpec {
    pub name: String,
    #[serde(default)]
    pub blurhash_field: Option<String>,
    #[serde(default)]
    pub size: Option<(u32, u32)>,
    #[serde(default)]
    pub prefetch_window: Option<(u32, u32)>,
    #[serde(default)]
    pub evict: Option<Evict>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioSpec {
    pub name: String,
    #[serde(default)]
    pub evict: Option<Evict>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MediaSpec {
    pub cache: CacheSpec,
    pub videos: Vec<VideoSpec>,
    pub images: Vec<ImageSpec>,
    pub audios: Vec<AudioSpec>,
}

impl MediaSpec {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("spec serializes")
    }

    pub fn from_json(text: &str) -> Result<MediaSpec, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn limits(&self) -> Limits {
        self.limits_for(self.videos.first())
    }

    pub(super) fn limits_for(&self, video: Option<&VideoSpec>) -> Limits {
        let prefetch = video.map(|video| video.prefetch.clone()).unwrap_or_default();
        let pool = video.map(|video| video.pool.clone()).unwrap_or_default();
        Limits {
            budget: self.cache.budget,
            min_budget: self.cache.min,
            shrink_to: f64::from(self.cache.shrink_to) / 100.0,
            memory_budget: self.cache.memory,
            slots: pool.slots,
            prefetch_from: prefetch.from,
            prefetch_to: prefetch.to,
            depth_seconds: prefetch.depth,
            timeout_seconds: self.cache.timeout,
            max_concurrent: self.cache.max_concurrent,
            dedupe_window_seconds: self.cache.dedupe_window,
            dedupe_max_entries: self.cache.dedupe_entries,
            strategy: self.cache.evict.into(),
        }
    }
}
