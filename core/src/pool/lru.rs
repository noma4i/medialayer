use super::ring::{Command, Owner, Ring};

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
        if !self.bind(slot, key.to_string(), Owner::Lru, None, now, &mut unloads, &mut loads) {
            return unloads;
        }
        let mutes = self.sync_mute();
        [unloads, loads, mutes].concat()
    }
}
