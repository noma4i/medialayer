use super::assembled::Assembled;
use super::deferred::Deferred;
use crate::api::Pressure;
use std::sync::Arc;

#[derive(uniffi::Object)]
pub struct PlayerEvents {
    assembled: Arc<Deferred<Assembled>>,
}

impl PlayerEvents {
    pub fn new(assembled: Arc<Deferred<Assembled>>) -> Arc<PlayerEvents> {
        Arc::new(PlayerEvents { assembled })
    }
}

#[uniffi::export]
impl PlayerEvents {
    pub fn ready(&self, slot: u32) {
        self.assembled.run(move |assembled| assembled.stage.ready(slot as usize));
    }

    pub fn visual_ready(&self, slot: u32) {
        self.assembled.run(move |assembled| assembled.stage.visual_ready(slot as usize));
    }

    pub fn stalled(&self, slot: u32, position_ms: u64) {
        self.assembled.run(move |assembled| assembled.stage.stalled(slot as usize, position_ms));
    }

    pub fn failed(&self, slot: u32, status: i32, detail: String) {
        self.assembled.run(move |assembled| assembled.stage.failed(slot as usize, status, detail));
    }

    pub fn ended(&self, slot: u32) {
        self.assembled.run(move |assembled| assembled.stage.ended(slot as usize));
    }

    pub fn attach(&self, key: String, view_token: u64) {
        self.assembled.run(move |assembled| assembled.stage.attach(&key, view_token));
    }

    pub fn detach(&self, key: String) {
        self.assembled.run(move |assembled| assembled.stage.detach(&key));
    }

    pub fn foreground(&self) {
        self.assembled.run(|assembled| assembled.signals.foreground());
    }

    pub fn background(&self) {
        self.assembled.run(|assembled| assembled.signals.background());
    }

    pub fn memory_warning(&self, level: Pressure) {
        self.assembled.run(move |assembled| assembled.signals.memory_warning(level));
    }
}
