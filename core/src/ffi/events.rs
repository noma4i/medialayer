use super::ingress::{Declared, PlayerEvent};
use super::layer::Closing;
use crate::api::{MediaError, Pressure};
use std::sync::{Arc, Weak};

/// Native player events. Every event is admitted by the typed input, so a flood closes the layer with an error instead of losing events silently.
#[derive(uniffi::Object)]
pub struct PlayerEvents {
    closing: Weak<Closing>,
}

impl PlayerEvents {
    pub(super) fn new(closing: &Arc<Closing>) -> Arc<PlayerEvents> {
        Arc::new(PlayerEvents { closing: Arc::downgrade(closing) })
    }

    fn declare(&self, command: Declared) {
        let Some(closing) = self.closing.upgrade() else { return };
        match closing.queue.submit(command) {
            Err(error @ MediaError::Overloaded { .. }) => closing.overflowed(error),
            _ => {}
        }
    }

    fn player(&self, event: PlayerEvent) {
        self.declare(Declared::Player(event));
    }
}

#[uniffi::export]
impl PlayerEvents {
    pub fn ready(&self, slot: u32, generation: u64) {
        self.player(PlayerEvent::Ready { slot, generation });
    }

    pub fn visual_ready(&self, slot: u32, generation: u64) {
        self.player(PlayerEvent::VisualReady { slot, generation });
    }

    pub fn stalled(&self, slot: u32, generation: u64, position_ms: u64) {
        self.player(PlayerEvent::Stalled { slot, generation, position_ms });
    }

    pub fn failed(&self, slot: u32, generation: u64, status: i32, detail: String) {
        self.player(PlayerEvent::Failed { slot, generation, status, detail });
    }

    pub fn ended(&self, slot: u32, generation: u64) {
        self.player(PlayerEvent::Ended { slot, generation });
    }

    pub fn foreground(&self) {
        self.declare(Declared::Lifecycle { foreground: true });
    }

    pub fn background(&self) {
        self.declare(Declared::Lifecycle { foreground: false });
    }

    pub fn memory_warning(&self, level: Pressure) {
        self.declare(Declared::Memory(level));
    }
}
