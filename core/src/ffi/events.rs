use super::assembled::Assembled;
use super::deferred::Deferred;
use crate::api::Pressure;
use std::sync::{Arc, Weak};

#[derive(uniffi::Object)]
pub struct PlayerEvents {
    assembled: Weak<Deferred<Assembled>>,
}

impl PlayerEvents {
    pub fn new(assembled: Arc<Deferred<Assembled>>) -> Arc<PlayerEvents> {
        Arc::new(PlayerEvents { assembled: Arc::downgrade(&assembled) })
    }

    fn declare(&self, work: impl FnOnce(&Assembled) + Send + 'static) {
        if let Some(assembled) = self.assembled.upgrade() {
            assembled.run(work);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inv_android_07_events_do_not_keep_a_closed_owner_alive() {
        let assembled = Deferred::<Assembled>::new();
        let weak = Arc::downgrade(&assembled);
        let events = PlayerEvents::new(assembled.clone());
        drop(assembled);
        assert!(weak.upgrade().is_none());
        events.ready(0);
        events.foreground();
    }
}

#[uniffi::export]
impl PlayerEvents {
    pub fn ready(&self, slot: u32) {
        self.declare(move |assembled| assembled.stage.ready(slot as usize));
    }

    pub fn visual_ready(&self, slot: u32) {
        self.declare(move |assembled| assembled.stage.visual_ready(slot as usize));
    }

    pub fn stalled(&self, slot: u32, position_ms: u64) {
        self.declare(move |assembled| assembled.stage.stalled(slot as usize, position_ms));
    }

    pub fn failed(&self, slot: u32, status: i32, detail: String) {
        self.declare(move |assembled| assembled.stage.failed(slot as usize, status, detail));
    }

    pub fn ended(&self, slot: u32) {
        self.declare(move |assembled| assembled.stage.ended(slot as usize));
    }

    pub fn attach(&self, key: String, view_token: u64) {
        self.declare(move |assembled| assembled.stage.attach(&key, view_token));
    }

    pub fn detach(&self, key: String) {
        self.declare(move |assembled| assembled.stage.detach(&key));
    }

    pub fn foreground(&self) {
        self.declare(|assembled| assembled.signals.foreground());
    }

    pub fn background(&self) {
        self.declare(|assembled| assembled.signals.background());
    }

    pub fn memory_warning(&self, level: Pressure) {
        self.declare(move |assembled| assembled.signals.memory_warning(level));
    }
}
