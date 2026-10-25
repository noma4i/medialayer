use super::layer::MemoryReleaser;
use crate::api::{follow_stats, AbortOnDrop, ControlPorts, Controls, MemoryRelease, Parts, Places, Routes, Signals, Stage, Suspendable, Tones};
use std::sync::Arc;
use tokio::runtime::Handle;

pub struct Assembled {
    pub controls: Arc<Controls>,
    pub signals: Arc<Signals>,
    pub stage: Stage,
    pub places: Arc<Places>,
    pub routes: Arc<Routes>,
    pub tones: Arc<Tones>,
    _stats_feed: AbortOnDrop,
}

struct ForeignRelease(Option<Arc<dyn MemoryReleaser>>);

impl MemoryRelease for ForeignRelease {
    fn drop_caches(&self) {
        if let Some(releaser) = &self.0 {
            releaser.drop_memory_caches();
        }
    }
}

impl Assembled {
    pub async fn close(&self) {
        self.stage.close().await;
    }

    pub fn new(handle: &Handle, parts: Parts, releaser: Option<Arc<dyn MemoryReleaser>>) -> Assembled {
        let release: Arc<dyn MemoryRelease> = Arc::new(ForeignRelease(releaser));
        let ports = ControlPorts {
            tones: parts.tones.clone(),
            cache: Arc::new(parts.cache),
            pool: Arc::new(parts.stage.clone()),
            prefetch: Arc::new(parts.prefetcher.clone()),
            settings: parts.settings,
            ledger: parts.ledger.clone(),
            device: parts.device,
            transport: Arc::new(parts.proxy.clone()),
            pool_settings: parts.pool,
            release: release.clone(),
        };
        let controls = Arc::new(Controls::new(handle.clone(), ports, parts.defaults, parts.limits, parts.rejected, parts.change.clone()));
        let suspendable: Vec<Arc<dyn Suspendable>> = vec![Arc::new(parts.prefetcher.clone()), Arc::new(parts.tones.clone())];
        let signals = Arc::new(Signals::new(
            handle,
            Arc::new(parts.stage.clone()),
            Arc::new(parts.prefetcher),
            Arc::new(parts.stage.clone()),
            release,
            suspendable,
            parts.ledger,
            parts.clock,
        ));
        let stats_feed = follow_stats(handle, Arc::downgrade(&controls), parts.change);
        Assembled { controls, signals, stage: parts.stage, places: parts.places, routes: parts.routes, tones: parts.tones, _stats_feed: stats_feed }
    }
}
