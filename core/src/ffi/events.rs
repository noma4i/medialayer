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

    fn declare_latest<P: Send + 'static>(&self, payload: P, name: fn(&P) -> String, work: impl FnOnce(&Assembled, P) + Send + 'static) {
        if let Some(assembled) = self.assembled.upgrade() {
            assembled.run_latest(payload, name, work);
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
        events.ready(0, 0);
        events.foreground();
    }
}

#[uniffi::export]
impl PlayerEvents {
    pub fn ready(&self, slot: u32, generation: u64) {
        self.declare(move |assembled| assembled.stage.ready(slot as usize, generation));
    }

    pub fn visual_ready(&self, slot: u32, generation: u64) {
        self.declare(move |assembled| assembled.stage.visual_ready(slot as usize, generation));
    }

    pub fn stalled(&self, slot: u32, generation: u64, position_ms: u64) {
        self.declare(move |assembled| assembled.stage.stalled(slot as usize, generation, position_ms));
    }

    pub fn failed(&self, slot: u32, generation: u64, status: i32, detail: String) {
        self.declare(move |assembled| assembled.stage.failed(slot as usize, generation, status, detail));
    }

    pub fn ended(&self, slot: u32, generation: u64) {
        self.declare(move |assembled| assembled.stage.ended(slot as usize, generation));
    }

    pub fn attach(&self, key: String, view_token: u64) {
        self.declare_latest(key, |key| format!("view:{key}"), move |assembled, key| assembled.stage.attach(&key, view_token));
    }

    pub fn detach(&self, key: String) {
        self.declare_latest(key, |key| format!("view:{key}"), move |assembled, key| assembled.stage.detach(&key));
    }

    pub fn foreground(&self) {
        self.declare_latest((), |_| "lifecycle".to_string(), |assembled, ()| assembled.signals.foreground());
    }

    pub fn background(&self) {
        self.declare_latest((), |_| "lifecycle".to_string(), |assembled, ()| assembled.signals.background());
    }

    pub fn memory_warning(&self, level: Pressure) {
        self.declare(move |assembled| assembled.signals.memory_warning(level));
    }
}
