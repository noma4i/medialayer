use super::ring::{Command, Holder, Ring};
use super::stats::ReleaseReason;
use crate::memory::{Held, Owner as MemoryOwner, Step, StepPlan};
use std::cmp::Reverse;

impl Ring {
    pub fn set_step(&mut self, step: Step) {
        self.step = step;
        self.ledger.set_pressure_step(step);
    }

    pub(super) fn effective_reach(&self) -> usize {
        StepPlan::for_step(self.step).reach_limit(self.reach)
    }

    pub(super) fn live(&self) -> usize {
        self.bindings.iter().flatten().count()
    }

    pub(super) fn admit(&mut self, slot: usize, page: Option<usize>, unloads: &mut Vec<Command>) -> Option<Held> {
        if self.live() >= StepPlan::for_step(self.step).slot_limit(self.slots) {
            return None;
        }
        let distance = page.map(|page| page.abs_diff(self.active));
        let ledger = self.ledger.clone();
        let bytes = self.config.slot_bytes_estimate;
        let mut freeing = |needed: u64| self.free_for(needed, distance, unloads);
        ledger.reserve_or_free(MemoryOwner::Slot(slot), bytes, &mut freeing).ok().map(Held::new)
    }

    pub(super) fn trim_to_limit(&mut self, unloads: &mut Vec<Command>) {
        let limit = StepPlan::for_step(self.step).slot_limit(self.slots);
        while self.live() > limit {
            let Some(victim) = self.limit_victim(limit == 0) else {
                return;
            };
            self.unbind(victim, ReleaseReason::Pressure, unloads);
        }
    }

    pub fn has_refused(&self) -> bool {
        !self.is_background && !self.refused.is_empty()
    }

    pub fn retry_refused(&mut self, now: u64, playing: Option<&str>) -> Vec<Command> {
        let is_blocked = self.hold.as_ref().is_some_and(|hold| hold.until.is_none_or(|until| now < until));
        if self.is_background || is_blocked {
            self.refused.clear();
            return Vec::new();
        }
        let mut unloads = Vec::new();
        let mut loads = Vec::new();
        let mut prerolls = Vec::new();
        let mut index = 0;
        while index < self.refused.len() {
            let (page, key) = self.refused.remove(index);
            if self.slot_of(&key).is_some() || !self.window.contains(&key) {
                continue;
            }
            let bound = self.acquire(Some(self.slot(page)), &mut unloads).filter(|slot| self.bind(*slot, key.clone(), Holder::Ring, Some(page), now, &mut unloads, &mut loads));
            let Some(slot) = bound else {
                self.refused.insert(index, (page, key));
                index += 1;
                continue;
            };
            if playing != Some(key.as_str()) {
                if let Some(binding) = self.bindings[slot].as_mut() {
                    binding.is_prerolled = true;
                }
                prerolls.push(Command::Preroll { slot });
            }
        }
        let retains = self.refresh_levels(now);
        let mutes = self.sync_mute();
        [unloads, loads, retains, mutes, prerolls].concat()
    }

    fn releasing_bytes(&mut self) -> u64 {
        self.releasing.retain(|held| held.strong_count() > 0);
        self.releasing.iter().filter_map(std::sync::Weak::upgrade).map(|held| held.bytes()).sum()
    }

    fn free_for(&mut self, needed: u64, distance: Option<usize>, unloads: &mut Vec<Command>) -> u64 {
        let waiting = self.refused.len() as u64 * self.config.slot_bytes_estimate;
        let mut freed = self.releasing_bytes().saturating_sub(waiting);
        while freed < needed {
            let Some(victim) = self.memory_victim(distance) else {
                break;
            };
            freed += self.bindings[victim].as_ref().map_or(0, |binding| binding.reservation.bytes());
            self.unbind(victim, ReleaseReason::Pressure, unloads);
        }
        freed
    }

    fn memory_victim(&self, distance: Option<usize>) -> Option<usize> {
        let candidates = self
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(slot, binding)| binding.as_ref().filter(|binding| !binding.is_playing && binding.owner != Holder::Pinned).map(|binding| (slot, binding)));
        let farther = |binding: &super::ring::Binding, than: usize| binding.page.is_none_or(|page| page.abs_diff(self.active) > than);
        let (_, victim) = match distance {
            None => candidates
                .filter(|(_, binding)| self.is_spare(binding))
                .map(|(slot, binding)| ((false, Reverse(0), binding.last_used), slot))
                .min()?,
            Some(than) => candidates
                .filter(|(_, binding)| binding.owner == Holder::Lru || farther(binding, than))
                .map(|(slot, binding)| {
                    let far = binding.page.map_or(usize::MAX, |page| page.abs_diff(self.active));
                    (((binding.owner != Holder::Lru), Reverse(far), binding.last_used), slot)
                })
                .min()?,
        };
        Some(victim)
    }

    fn limit_victim(&self, is_total: bool) -> Option<usize> {
        let spare = self
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(slot, binding)| binding.as_ref().filter(|binding| !binding.is_playing && binding.owner != Holder::Pinned).map(|binding| (slot, binding)))
            .map(|(slot, binding)| ((Reverse(binding.page.map_or(usize::MAX, |page| page.abs_diff(self.active))), binding.last_used), slot))
            .min()
            .map(|(_, slot)| slot);
        spare.or_else(|| if is_total { self.bindings.iter().position(Option::is_some) } else { None })
    }
}
