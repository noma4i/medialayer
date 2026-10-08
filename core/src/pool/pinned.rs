use super::ring::{Command, Holder, Ring};

impl Ring {
    pub fn pin(&mut self, now: u64, key: &str) -> Vec<Command> {
        if let Some(slot) = self.slot_of(key) {
            if let Some(binding) = self.bindings[slot].as_mut() {
                binding.owner = Holder::Pinned;
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
        if !self.bind(slot, key.to_string(), Holder::Pinned, None, now, &mut unloads, &mut loads) {
            return unloads;
        }
        let mutes = self.sync_mute();
        [unloads, loads, mutes].concat()
    }

    pub fn unpin(&mut self, key: &str) {
        let Some(binding) = self.slot_of(key).and_then(|slot| self.bindings[slot].as_mut()) else {
            return;
        };
        if binding.owner == Holder::Pinned {
            binding.owner = if binding.page.is_some() { Holder::Ring } else { Holder::Lru };
        }
    }
}
