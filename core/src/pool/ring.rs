use super::config::PoolConfig;
use crate::memory::{Held, Ledger, Reservation, Step};
use super::retention::{Retention, SlotState};
use super::stats::{PoolCounters, ReleaseReason};
use crate::pages::PageKeys;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::sync::Weak;

#[derive(Debug, Clone)]
pub enum Command {
    Load { slot: usize, key: String, held: Held },
    Unload { slot: usize },
    Dispose { slot: usize },
    Preroll { slot: usize },
    Play { slot: usize },
    Pause { slot: usize },
    Mute { slot: usize },
    Unmute { slot: usize },
    Retain { slot: usize, level: Retention },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    Ring,
    Lru,
    Pinned,
}

#[derive(Debug)]
pub(super) struct Binding {
    pub key: String,
    pub owner: Holder,
    pub page: Option<usize>,
    pub is_playing: bool,
    pub is_peeking: bool,
    pub is_prerolled: bool,
    pub level: Retention,
    pub positional: Retention,
    pub is_trimmed: bool,
    pub since: u64,
    pub last_used: u64,
    pub reservation: Held,
}

#[derive(Debug, Clone)]
pub(super) struct Hold {
    pub window: Vec<String>,
    pub until: Option<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct Saved {
    pub key: String,
    pub owner: Holder,
    pub page: Option<usize>,
}

#[derive(Debug)]
pub struct Ring {
    pub(super) slots: usize,
    pub(super) reach: usize,
    pub(super) config: PoolConfig,
    pub(super) bindings: Vec<Option<Binding>>,
    pub(super) native_muted: Vec<bool>,
    pub(super) uses: Vec<u32>,
    pub(super) muted_all: bool,
    pub(super) muted_keys: HashSet<String>,
    pub(super) active: usize,
    pub(super) window: Vec<String>,
    pub(super) hold: Option<Hold>,
    pub(super) is_background: bool,
    pub(super) restore: VecDeque<Saved>,
    pub(super) counters: PoolCounters,
    pub(super) ledger: Ledger,
    pub(super) step: Step,
    pub(super) releasing: Vec<Weak<Reservation>>,
    pub(super) refused: Vec<(usize, String)>,
    pub(super) is_closed: bool,
}

impl Ring {
    pub fn new(config: PoolConfig) -> Self {
        Self::with_ledger(config, Ledger::unbounded())
    }

    pub fn with_ledger(config: PoolConfig, ledger: Ledger) -> Self {
        let slots = config.slots.max(2 * config.reach + 1);
        Self {
            slots,
            reach: config.reach,
            bindings: (0..slots).map(|_| None).collect(),
            native_muted: vec![config.start_muted; slots],
            uses: vec![0; slots],
            muted_all: config.start_muted,
            muted_keys: HashSet::new(),
            active: 0,
            window: Vec::new(),
            hold: None,
            is_background: false,
            restore: VecDeque::new(),
            counters: PoolCounters::default(),
            ledger,
            step: Step::Normal,
            releasing: Vec::new(),
            refused: Vec::new(),
            is_closed: false,
            config,
        }
    }

    pub fn slots(&self) -> usize {
        self.slots
    }

    pub fn set_slots(&mut self, slots: usize) -> Vec<Command> {
        let slots = slots.max(2 * self.reach + 1);
        let mut unloads = Vec::new();
        for slot in slots..self.slots {
            self.unbind(slot, ReleaseReason::Explicit, &mut unloads);
            unloads.push(Command::Dispose { slot });
        }
        self.bindings.resize_with(slots, || None);
        self.native_muted.resize(slots, self.muted_all);
        self.uses.resize(slots, 0);
        self.slots = slots;
        self.config.slots = slots;
        unloads
    }

    pub fn slot(&self, page: usize) -> usize {
        page % self.slots
    }

    pub fn pages(&self, active: usize, count: usize) -> Range<usize> {
        if count == 0 {
            return 0..0;
        }
        let reach = self.effective_reach();
        active.saturating_sub(reach)..active.saturating_add(reach).saturating_add(1).min(count)
    }

    pub fn plan(&mut self, now: u64, active: usize, playback: Option<usize>, keys: &(impl PageKeys + ?Sized)) -> Vec<Command> {
        self.active = active;
        self.refused.clear();
        if self.is_background {
            return Vec::new();
        }
        let wanted = self.pages(active, keys.count());
        let pages: HashMap<&str, usize> =
            wanted.clone().filter_map(|page| keys.key(page).map(|key| (key, page))).collect();
        let window: Vec<String> = wanted.clone().filter_map(|page| keys.key(page).map(str::to_string)).collect();
        if self.hold.as_ref().is_some_and(|hold| hold.window != window || hold.until.is_some_and(|until| now >= until)) {
            self.hold = None;
        }
        let is_blocked = self.hold.is_some();
        self.window = window;

        let mut unloads = Vec::new();
        let mut staged_loads: Vec<(usize, Vec<Command>)> = Vec::new();
        for slot in 0..self.slots {
            let Some(binding) = self.bindings[slot].as_mut() else {
                continue;
            };
            binding.page = pages.get(binding.key.as_str()).copied();
            if binding.owner == Holder::Lru && binding.page.is_some() {
                binding.owner = Holder::Ring;
            }
            if binding.owner == Holder::Ring && binding.page.is_none() {
                self.unbind(slot, ReleaseReason::SourceChange, &mut unloads);
            }
        }
        self.trim_to_limit(&mut unloads);
        let mut nearest_first: Vec<usize> = wanted.clone().collect();
        nearest_first.sort_by_key(|page| (page.abs_diff(active), *page < active));
        for page in nearest_first {
            let Some(key) = keys.key(page) else {
                continue;
            };
            if is_blocked || self.slot_of(key).is_some() {
                continue;
            }
            let Some(slot) = self.acquire(Some(self.slot(page)), &mut unloads) else {
                self.refused.push((page, key.to_string()));
                continue;
            };
            let mut staged = Vec::new();
            if !self.bind(slot, key.to_string(), Holder::Ring, Some(page), now, &mut unloads, &mut staged) {
                self.refused.push((page, key.to_string()));
            }
            staged_loads.push((page, staged));
        }
        staged_loads.sort_by_key(|(page, _)| *page);
        let loads: Vec<Command> = staged_loads.into_iter().flat_map(|(_, staged)| staged).collect();

        let playback_key = playback.filter(|page| wanted.contains(page)).and_then(|page| keys.key(page).map(str::to_string));
        let mut pauses = Vec::new();
        let mut plays = Vec::new();
        let mut prerolls = Vec::new();
        for slot in 0..self.slots {
            let Some(binding) = self.bindings[slot].as_mut() else {
                continue;
            };
            let should_play = playback_key.as_deref() == Some(binding.key.as_str());
            let is_ring = binding.owner == Holder::Ring;
            if binding.is_playing && !should_play && (is_ring || playback_key.is_some()) {
                binding.is_playing = false;
                pauses.push(Command::Pause { slot });
            }
            if !binding.is_playing && should_play {
                binding.is_playing = true;
                if !std::mem::take(&mut binding.is_peeking) {
                    plays.push(Command::Play { slot });
                }
            }
            if !should_play && is_ring && !binding.is_prerolled {
                binding.is_prerolled = true;
                prerolls.push(Command::Preroll { slot });
            }
        }
        let retains = self.refresh_levels(now);
        let mutes = self.sync_mute();
        [unloads, loads, retains, mutes, pauses, plays, prerolls].concat()
    }

    pub fn states(&self) -> Vec<SlotState> {
        self.bindings
            .iter()
            .enumerate()
            .map(|(slot, binding)| match binding {
                None => SlotState { slot, retention: Retention::Cold, is_playing: false, is_pinned: false, distance: usize::MAX },
                Some(binding) => SlotState {
                    slot,
                    retention: binding.level,
                    is_playing: binding.is_playing,
                    is_pinned: binding.owner == Holder::Pinned,
                    distance: binding.page.map_or(usize::MAX, |page| page.abs_diff(self.active)),
                },
            })
            .collect()
    }

    pub fn retention_of(&self, slot: usize) -> Option<Retention> {
        self.bindings.get(slot)?.as_ref().map(|binding| if binding.is_playing { Retention::Hot } else { binding.level })
    }

    pub fn slot_of(&self, key: &str) -> Option<usize> {
        self.bindings.iter().position(|binding| binding.as_ref().is_some_and(|binding| binding.key == key))
    }

    pub fn key_of(&self, slot: usize) -> Option<&str> {
        self.bindings.get(slot)?.as_ref().map(|binding| binding.key.as_str())
    }

    pub(super) fn is_spare(&self, binding: &Binding) -> bool {
        binding.owner == Holder::Lru && !binding.is_playing && !self.window.contains(&binding.key)
    }

    pub(super) fn acquire(&mut self, preferred: Option<usize>, unloads: &mut Vec<Command>) -> Option<usize> {
        if let Some(slot) = preferred.filter(|slot| self.bindings[*slot].is_none()) {
            return Some(slot);
        }
        if let Some(slot) = self.bindings.iter().position(Option::is_none) {
            return Some(slot);
        }
        let (_, victim) = self
            .bindings
            .iter()
            .enumerate()
            .filter_map(|(slot, binding)| binding.as_ref().filter(|binding| self.is_spare(binding)).map(|binding| (binding.last_used, slot)))
            .min()?;
        self.unbind(victim, ReleaseReason::SourceChange, unloads);
        Some(victim)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn bind(
        &mut self,
        slot: usize,
        key: String,
        owner: Holder,
        page: Option<usize>,
        now: u64,
        unloads: &mut Vec<Command>,
        loads: &mut Vec<Command>,
    ) -> bool {
        let Some(reservation) = self.admit(slot, page, unloads) else {
            return false;
        };
        if self.uses[slot] > 0 {
            self.counters.reuses += 1;
        }
        self.uses[slot] += 1;
        loads.push(Command::Load { slot, key: key.clone(), held: reservation.clone() });
        self.bindings[slot] = Some(Binding {
            key,
            owner,
            page,
            is_playing: false,
            is_peeking: false,
            is_prerolled: false,
            level: Retention::Warm,
            positional: Retention::Warm,
            is_trimmed: false,
            since: now,
            last_used: now,
            reservation,
        });
        true
    }

    pub(super) fn take_binding(&mut self, slot: usize, reason: ReleaseReason) -> Option<Binding> {
        let binding = self.bindings[slot].take()?;
        self.counters.released.count(reason);
        self.releasing.push(binding.reservation.downgrade());
        Some(binding)
    }

    pub(super) fn unbind(&mut self, slot: usize, reason: ReleaseReason, out: &mut Vec<Command>) {
        if self.take_binding(slot, reason).is_some() {
            out.push(Command::Unload { slot });
        }
    }

    pub(super) fn refresh_levels(&mut self, now: u64) -> Vec<Command> {
        let next = self.active.saturating_add(1);
        let mut out = Vec::new();
        for (slot, binding) in self.bindings.iter_mut().enumerate() {
            let Some(binding) = binding else {
                continue;
            };
            let desired = if binding.is_playing || binding.page == Some(next) { Retention::Hot } else { Retention::Warm };
            let is_changed = desired != binding.positional || (binding.is_playing && binding.level != Retention::Hot);
            if !is_changed {
                continue;
            }
            binding.positional = desired;
            binding.level = desired;
            binding.since = now;
            if desired == Retention::Hot && binding.is_trimmed {
                binding.is_trimmed = false;
                out.push(Command::Retain { slot, level: Retention::Hot });
            }
        }
        out
    }
}
