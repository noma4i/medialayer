use super::ring::{Command, Holder, Ring};
use super::stats::ReleaseReason;
use std::cmp::Reverse;

impl Ring {
    pub fn claim(&mut self, now: u64, key: &str) -> Vec<Command> {
        if self.slot_of(key).is_some() || self.is_background {
            return self.touch(now, key);
        }
        let mut unloads = Vec::new();
        let slot = self.acquire(None, &mut unloads).or_else(|| {
            let victim = self.bindings.iter().enumerate().filter_map(|(slot, binding)| {
                binding.as_ref().filter(|binding| !binding.is_playing && binding.owner != Holder::Pinned)
                    .map(|binding| ((Reverse(binding.page.map_or(usize::MAX, |page| page.abs_diff(self.active))), binding.last_used), slot))
            }).min().map(|(_, slot)| slot)?;
            self.unbind(victim, ReleaseReason::SourceChange, &mut unloads);
            Some(victim)
        });
        let Some(slot) = slot else { return unloads; };
        let mut loads = Vec::new();
        if !self.bind(slot, key.to_string(), Holder::Lru, None, now, &mut unloads, &mut loads) {
            return unloads;
        }
        let mutes = self.sync_mute();
        [unloads, loads, mutes].concat()
    }

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
