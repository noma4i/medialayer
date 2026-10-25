use super::error::MediaError;
use super::hub::{Deliver, Hub, Subscription};
use super::assemble::{pool_config, PoolSettings};
use super::ports::{CacheControl, MemoryRelease, PoolControl, PrefetchControl, SettingsStore, TransportControl};
use super::types::{Kind, MediaStats};
use crate::change::Change;
use crate::limits::Limits;
use crate::memory::{effective_ceiling, DeviceMemory, Ledger, ShareTuning};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::runtime::Handle;
use tokio::sync::Mutex as AsyncMutex;

pub const LIMITS_KEY: &str = "medialayer.limits";

pub struct ControlPorts {
    pub tones: Arc<super::tones::Tones>,
    pub cache: Arc<dyn CacheControl>,
    pub pool: Arc<dyn PoolControl>,
    pub prefetch: Arc<dyn PrefetchControl>,
    pub settings: Arc<dyn SettingsStore>,
    pub ledger: Ledger,
    pub device: DeviceMemory,
    pub transport: Arc<dyn TransportControl>,
    pub pool_settings: PoolSettings,
    pub release: Arc<dyn MemoryRelease>,
}

pub struct Controls {
    ports: ControlPorts,
    defaults: Limits,
    current: AsyncMutex<Limits>,
    diagnostics: Mutex<Vec<String>>,
    published: Mutex<Option<MediaStats>>,
    stats_hub: Hub<MediaStats>,
    change: Arc<Change>,
    wakes: AtomicU64,
    closed: AtomicBool,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Controls {
    pub fn new(handle: Handle, ports: ControlPorts, defaults: Limits, limits: Limits, rejected: Option<String>, change: Arc<Change>) -> Controls {
        Controls {
            ports,
            defaults,
            current: AsyncMutex::new(limits),
            diagnostics: Mutex::new(rejected.into_iter().collect()),
            published: Mutex::new(None),
            stats_hub: Hub::new(handle),
            change,
            wakes: AtomicU64::new(0),
            closed: AtomicBool::new(false),
        }
    }

    pub fn count_wake(&self) {
        self.wakes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn wakes(&self) -> u64 {
        self.wakes.load(Ordering::Relaxed)
    }

    pub async fn limits(&self) -> Limits {
        *self.current.lock().await
    }

    pub async fn set_limits(&self, limits: Limits) -> Result<(), MediaError> {
        limits.validate()?;
        let mut current = self.current.lock().await;
        self.apply(limits).await?;
        let settings = self.ports.settings.clone();
        let encoded = limits.encode();
        let _ = tokio::task::spawn_blocking(move || settings.write(LIMITS_KEY.to_string(), encoded)).await;
        *current = limits;
        drop(current);
        self.publish_stats().await;
        Ok(())
    }

    pub async fn reset_limits(&self) -> Result<(), MediaError> {
        let mut current = self.current.lock().await;
        self.apply(self.defaults).await?;
        let settings = self.ports.settings.clone();
        let _ = tokio::task::spawn_blocking(move || settings.remove(LIMITS_KEY.to_string())).await;
        *current = self.defaults;
        drop(current);
        self.publish_stats().await;
        Ok(())
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    pub async fn stats(&self) -> MediaStats {
        self.stats_with(self.ports.pool.stats()).await
    }

    /// Stats over a pool snapshot the media executor took at its own barrier.
    pub async fn stats_with(&self, pool: crate::pool::PoolStats) -> MediaStats {
        let diagnostics = locked(&self.diagnostics).clone();
        MediaStats::collect(&self.ports.cache.stats().await, &pool, diagnostics)
    }

    pub async fn clear(&self, kind: Option<Kind>) {
        self.ports.prefetch.forget_loaded(None);
        self.ports.cache.clear(kind.map(Into::into)).await;
        self.ports.transport.forget_playlists(None);
        if matches!(kind, None | Some(Kind::Image)) {
            let release = self.ports.release.clone();
            let _ = tokio::task::spawn_blocking(move || release.drop_caches()).await;
        }
        self.publish_stats().await;
    }

    pub async fn purge(&self, url: &str) {
        self.ports.prefetch.forget_loaded(Some(url));
        self.ports.tones.forget(url);
        self.ports.cache.purge(url).await;
        self.ports.transport.forget_playlists(Some(url));
        self.publish_stats().await;
    }

    pub async fn evict_now(&self) {
        self.ports.cache.evict_now().await;
        self.publish_stats().await;
    }

    #[cfg(test)]
    pub fn published_count(&self) -> u64 {
        self.stats_hub.published()
    }

    pub fn subscribe_stats(&self, deliver: Deliver<MediaStats>) -> Subscription {
        *locked(&self.published) = None;
        let subscription = self.stats_hub.subscribe(deliver);
        self.change.want();
        subscription
    }

    pub fn has_stats_listener(&self) -> bool {
        self.stats_hub.listener_count() > 0
    }

    pub async fn publish_stats(&self) {
        if self.closed.load(Ordering::SeqCst) || self.stats_hub.listener_count() == 0 {
            return;
        }
        let stats = self.stats().await;
        {
            let mut published = locked(&self.published);
            if published.as_ref() == Some(&stats) {
                return;
            }
            *published = Some(stats.clone());
        }
        self.stats_hub.publish(stats);
    }

    async fn apply(&self, limits: Limits) -> Result<(), MediaError> {
        self.ports.cache.set_limits(limits).await?;
        self.ports.prefetch.set_limits(&limits);
        self.ports.transport.set_limits(&limits);
        self.ports.pool.set_slots(pool_config(&limits, self.ports.device, &self.ports.pool_settings).slots);
        self.ports.ledger.set_ceiling(effective_ceiling(limits.memory_budget, self.ports.device, ShareTuning::default()));
        Ok(())
    }
}
