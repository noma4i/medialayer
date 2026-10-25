use super::retention::{relieve, Directive, Pressure, Retention};
use super::ring::{Command, Hold, Ring, Saved};
use super::stats::ReleaseReason;

impl Ring {
    pub fn release_under(&mut self, now: u64, pressure: Pressure) -> Vec<Directive> {
        let directives = relieve(pressure, &self.states());
        let mut is_released = false;
        for directive in &directives {
            if let Directive::Slot(Command::Unload { slot }) = directive {
                is_released |= self.take_binding(*slot, ReleaseReason::Pressure).is_some();
            }
        }
        if is_released {
            self.hold = Some(Hold { window: self.window.clone(), until: Some(now.saturating_add(self.config.hold_off_ms)) });
        }
        directives
    }

    pub fn release(&mut self, key: &str) -> Vec<Command> {
        let mut out = Vec::new();
        if let Some(slot) = self.slot_of(key) {
            self.unbind(slot, ReleaseReason::Explicit, &mut out);
        }
        out
    }

    pub fn release_all(&mut self) -> Vec<Command> {
        let mut out = Vec::new();
        for slot in 0..self.slots {
            self.unbind(slot, ReleaseReason::Explicit, &mut out);
        }
        self.muted_keys.clear();
        self.restore.clear();
        self.hold = None;
        out
    }

    pub fn background(&mut self) -> Vec<Directive> {
        if self.is_background {
            return Vec::new();
        }
        self.is_background = true;
        let selected = self
            .bindings
            .iter()
            .position(|binding| binding.as_ref().is_some_and(|binding| binding.is_playing))
            .or_else(|| self.bindings.iter().position(|binding| binding.as_ref().is_some_and(|binding| binding.page == Some(self.active))));
        let mut out = vec![Directive::StopPrefetch];
        let mut saved = Vec::new();
        for slot in (0..self.slots).filter(|slot| Some(*slot) != selected) {
            let Some(binding) = self.take_binding(slot, ReleaseReason::Background) else {
                continue;
            };
            let distance = binding.page.map_or(usize::MAX, |page| page.abs_diff(self.active));
            saved.push((distance, Saved { key: binding.key, owner: binding.owner, page: binding.page }));
            out.push(Directive::Slot(Command::Unload { slot }));
        }
        saved.sort_by_key(|(distance, _)| *distance);
        self.restore = saved.into_iter().map(|(_, saved)| saved).collect();
        out
    }

    pub fn close(&mut self) -> Vec<Command> {
        let commands = self.release_all();
        self.is_closed = true;
        self.is_background = true;
        commands
    }

    pub fn foreground(&mut self) {
        self.is_background = self.is_closed;
    }

    pub fn next_due(&self) -> Option<u64> {
        if self.is_background {
            return None;
        }
        let trims = self.bindings.iter().flatten().filter(|binding| !binding.is_playing && binding.owner != super::ring::Holder::Pinned);
        trims.map(|binding| binding.since.saturating_add(self.config.trim_delay_ms)).min()
    }

    pub fn is_restoring(&self) -> bool {
        !self.is_background && !self.restore.is_empty()
    }

    pub fn restore_len(&self) -> usize {
        self.restore.len()
    }

    pub fn is_background(&self) -> bool {
        self.is_background
    }

    pub fn tick(&mut self, now: u64) -> Vec<Command> {
        if self.is_background {
            return Vec::new();
        }
        let mut out = self.restore_one(now);
        let mut is_released = false;
        for slot in 0..self.slots {
            let Some(binding) = self.bindings[slot].as_mut() else {
                continue;
            };
            let is_exempt = binding.is_playing || binding.owner == super::ring::Holder::Pinned;
            if is_exempt || now.saturating_sub(binding.since) < self.config.trim_delay_ms {
                continue;
            }
            if binding.level == Retention::Hot {
                binding.level = Retention::Warm;
                binding.since = now;
                binding.is_trimmed = true;
                out.push(Command::Retain { slot, level: Retention::Warm });
            } else {
                self.unbind(slot, ReleaseReason::Timer, &mut out);
                is_released = true;
            }
        }
        if is_released {
            self.hold = Some(Hold { window: self.window.clone(), until: None });
        }
        out
    }

    fn restore_one(&mut self, now: u64) -> Vec<Command> {
        while let Some(saved) = self.restore.pop_front() {
            if self.slot_of(&saved.key).is_some() {
                continue;
            }
            let mut unloads = Vec::new();
            let mut loads = Vec::new();
            let preferred = saved.page.map(|page| self.slot(page));
            let Some(slot) = self.acquire(preferred, &mut unloads) else {
                continue;
            };
            let is_bound = self.bind(slot, saved.key.clone(), saved.owner, saved.page, now, &mut unloads, &mut loads);
            if !is_bound {
                self.restore.push_front(saved);
                return unloads;
            }
            let mutes = self.sync_mute();
            return [unloads, loads, mutes].concat();
        }
        Vec::new()
    }
}
