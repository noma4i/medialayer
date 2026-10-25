use crate::cache;
use crate::image_size::ImageSize;
use crate::limits::{Limits, LimitsError};
use crate::memory::Step;
use crate::pool::{Directive, Pressure, PoolStats};
use async_trait::async_trait;

#[uniffi::export(with_foreign)]
pub trait SettingsStore: Send + Sync {
    fn read(&self, key: String) -> Option<String>;
    fn write(&self, key: String, value: String);
    fn remove(&self, key: String);
}

#[async_trait]
pub trait CacheControl: Send + Sync {
    async fn stats(&self) -> cache::CacheStats;
    async fn set_limits(&self, limits: Limits) -> Result<(), LimitsError>;
    async fn clear(&self, kind: Option<cache::Kind>);
    async fn purge(&self, url: &str);
    async fn evict_now(&self);
    async fn contains(&self, key: &str) -> bool;
    fn image_size(&self, key: &str) -> Option<ImageSize>;
    fn set_image_size(&self, key: &str, size: ImageSize) -> bool;
}

pub trait PoolControl: Send + Sync {
    fn stats(&self) -> PoolStats;
    fn set_step(&self, step: Step);
    fn release_under(&self, now_ms: u64, pressure: Pressure) -> Vec<Directive>;
    fn background(&self) -> Vec<Directive>;
    fn foreground(&self);
    fn set_slots(&self, slots: usize);
}

pub trait TransportControl: Send + Sync {
    fn set_limits(&self, limits: &Limits);
    fn forget_playlists(&self, url: Option<&str>);
}

pub trait PrefetchControl: Send + Sync {
    fn stop(&self);
    fn forget_loaded(&self, url: Option<&str>);
    fn set_limits(&self, limits: &Limits);
}

pub trait Suspendable: Send + Sync {
    fn suspend(&self);
    fn resume(&self);
}

pub trait MemoryRelease: Send + Sync {
    fn drop_caches(&self);
}

pub trait SlotDirectives: Send + Sync {
    fn apply(&self, directives: Vec<Directive>);
}
