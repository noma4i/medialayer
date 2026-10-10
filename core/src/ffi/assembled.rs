use super::layer::MemoryReleaser;
use crate::api::{spawn_pulse, AbortOnDrop, ControlPorts, Controls, MemoryRelease, Parts, Places, Routes, Signals, Stage};
use std::sync::Arc;
use tokio::runtime::Handle;

pub struct Assembled {
    pub controls: Arc<Controls>,
    pub signals: Arc<Signals>,
    pub stage: Stage,
    pub places: Arc<Places>,
    pub routes: Arc<Routes>,
    _pulse: AbortOnDrop,
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
    pub fn new(handle: &Handle, parts: Parts, releaser: Option<Arc<dyn MemoryReleaser>>) -> Assembled {
        let release: Arc<dyn MemoryRelease> = Arc::new(ForeignRelease(releaser));
        let ports = ControlPorts {
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
        let controls = Arc::new(Controls::new(handle.clone(), ports, parts.defaults));
        let signals = Arc::new(Signals::new(
            handle,
            Arc::new(parts.stage.clone()),
            Arc::new(parts.prefetcher),
            Arc::new(parts.stage.clone()),
            release,
            parts.ledger,
            parts.clock,
        ));
        let warm = controls.clone();
        handle.spawn(async move {
            warm.limits().await;
        });
        let pulse = spawn_pulse(handle, Arc::downgrade(&controls), parts.stats_interval);
        Assembled { controls, signals, stage: parts.stage, places: parts.places, routes: parts.routes, _pulse: pulse }
    }
}
