use super::ring::{Command, Holder, Ring};
use super::stats::ReleaseReason;
use crate::memory::{Owner as MemoryOwner, Reservation, Step, StepPlan};
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

    pub(super) fn admit(&mut self, slot: usize, page: Option<usize>, unloads: &mut Vec<Command>) -> Option<Reservation> {
        if self.live() >= StepPlan::for_step(self.step).slot_limit(self.slots) {
            return None;
        }
        let distance = page.map(|page| page.abs_diff(self.active));
        let ledger = self.ledger.clone();
        let bytes = self.config.slot_bytes_estimate;
        let mut freeing = |needed: u64| self.free_for(needed, distance, unloads);
        ledger.reserve_or_free(MemoryOwner::Slot(slot), bytes, &mut freeing).ok()
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

    fn free_for(&mut self, needed: u64, distance: Option<usize>, unloads: &mut Vec<Command>) -> u64 {
        let mut freed = 0;
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
                .filter(|(_, binding)| binding.owner == Holder::Lru)
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
