use super::ring::{Command, Holder, Ring};
use super::stats::ReleaseReason;

impl Ring {
    pub fn touch(&mut self, now: u64, key: &str) -> Vec<Command> {
        if let Some(slot) = self.slot_of(key) {
            if let Some(binding) = self.bindings[slot].as_mut() {
                binding.last_used = now;
            }
            return Vec::new();
        }
        if self.is_background {
            return Vec::new();
        }
        let mut unloads = Vec::new();
        let mut loads = Vec::new();
        let Some(slot) = self.acquire(None, &mut unloads) else {
            return Vec::new();
        };
        if !self.bind(slot, key.to_string(), Holder::Lru, None, now, &mut unloads, &mut loads) {
            return unloads;
        }
        let mutes = self.sync_mute();
        [unloads, loads, mutes].concat()
    }

    pub fn untouch(&mut self, key: &str) -> Vec<Command> {
        let Some(slot) = self.slot_of(key).filter(|slot| self.bindings[*slot].as_ref().is_some_and(|binding| binding.owner == Holder::Lru)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        self.unbind(slot, ReleaseReason::Explicit, &mut out);
        out
    }
}
