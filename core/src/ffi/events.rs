use crate::api::{Pressure, Signals, Stage};
use std::sync::Arc;

#[derive(uniffi::Object)]
pub struct PlayerEvents {
    stage: Stage,
    signals: Arc<Signals>,
}

impl PlayerEvents {
    pub fn new(stage: Stage, signals: Arc<Signals>) -> Arc<PlayerEvents> {
        Arc::new(PlayerEvents { stage, signals })
    }
}

#[uniffi::export]
impl PlayerEvents {
    pub fn ready(&self, slot: u32) {
        self.stage.ready(slot as usize);
    }

    pub fn visual_ready(&self, slot: u32) {
        self.stage.visual_ready(slot as usize);
    }

    pub fn stalled(&self, slot: u32, position_ms: u64) {
        self.stage.stalled(slot as usize, position_ms);
    }

    pub fn failed(&self, slot: u32, status: i32, detail: String) {
        self.stage.failed(slot as usize, status, detail);
    }

    pub fn ended(&self, slot: u32) {
        self.stage.ended(slot as usize);
    }

    pub fn foreground(&self) {
        self.signals.foreground();
    }

    pub fn background(&self) {
        self.signals.background();
    }

    pub fn memory_warning(&self, level: Pressure) {
        self.signals.memory_warning(level);
    }
}
