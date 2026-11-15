use super::positions::ResumePositions;
use crate::api::slots::SlotCall;
use crate::api::source::{MediaItem, PlaybackRoute};
use crate::memory::Held;
use crate::pages::PageKeys;
use crate::pool::{Command, Ring};
use crate::session::{self, Action, Context, Election, RecoveryState};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, MutexGuard};
use super::Inner;
use crate::api::guard::AbortOnDrop;

pub(super) struct Loaded {
    pub(super) held: Held,
    pub(super) slot: usize,
    pub(super) generation: u64,
    pub(super) recovery: RecoveryState,
    pub(super) source: MediaItem,
    pub(super) route: PlaybackRoute,
    pub(super) seeked: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct LoadId {
    pub(super) key: String,
    pub(super) slot: usize,
    pub(super) generation: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Work {
    Watchdog,
    Retry,
    Foreground,
    Repair,
    Purge,
}

impl Work {
    pub(super) fn is_timer(self) -> bool {
        matches!(self, Work::Watchdog | Work::Retry | Work::Foreground)
    }
}

pub(super) struct Armed {
    pub(super) generation: u64,
    pub(super) work: Work,
    pub(super) token: u64,
    pub(super) _task: AbortOnDrop,
}

pub(super) struct Declaration {
    pub(super) token: Arc<()>,
    pub(super) check: Option<AbortOnDrop>,
    #[cfg(test)]
    pub(super) observed: Option<tokio::task::AbortHandle>,
}

#[cfg(test)]
impl Drop for Declaration {
    fn drop(&mut self) {
        if let Some(observed) = &self.observed { observed.abort(); }
    }
}

#[derive(Default)]
pub(super) struct Effects {
    pub(super) calls: Vec<(usize, SlotCall)>,
    pub(super) actions: Vec<(LoadId, Action)>,
    pub(super) unloaded: Vec<String>,
    pub(super) resumed: Vec<(usize, String, u64)>,
    pub(super) parked: Vec<Parked>,
}

pub(super) struct Parked {
    pub(super) slot: usize,
    pub(super) key: String,
    pub(super) releases: u64,
    pub(super) seeked: Option<u64>,
}

pub(super) struct StageState {
    pub(super) ring: Ring,
    pub(super) election: Election,
    pub(super) leases: HashMap<String, u64>,
    pub(super) keys_by_lease: HashMap<u64, String>,
    pub(super) loaded: HashMap<String, Loaded>,
    pub(super) slot_keys: HashMap<usize, String>,
    pub(super) tasks: HashMap<String, Vec<Armed>>,
    pub(super) foreground: Option<(u64, AbortOnDrop)>,
    pub(super) catalog: HashMap<String, MediaItem>,
    pub(super) sliced: HashSet<String>,
    pub(super) views: HashMap<String, u64>,
    pub(super) shown: HashMap<String, usize>,
    pub(super) declarations: HashMap<String, Declaration>,
    pub(super) rates: HashMap<String, f32>,
    pub(super) positions: ResumePositions,
    pub(super) window: Vec<Option<MediaItem>>,
    pub(super) image_windows: HashMap<String, Vec<Vec<MediaItem>>>,
    pub(super) is_restore_requested: bool,
    pub(super) is_room_wanted: bool,
    pub(super) refusal_room: u64,
    pub(super) pending_wanted: HashSet<String>,
    pub(super) loads: u64,
    pub(super) releases: u64,
}


impl StageState {
    pub(super) fn leave(&mut self, key: &str) -> bool {
        match self.shown.get_mut(key) {
            Some(count) if *count > 1 => {
                *count -= 1;
                true
            }
            _ => {
                self.shown.remove(key);
                false
            }
        }
    }

    pub(super) fn lease_for(&mut self, key: &str) -> u64 {
        if let Some(lease) = self.leases.get(key) {
            return *lease;
        }
        let lease = self.election.register(key, Context::Ring.policy());
        self.leases.insert(key.to_string(), lease);
        self.keys_by_lease.insert(lease, key.to_string());
        self.refresh_pending(key);
        lease
    }

    pub(super) fn forget_lease(&mut self, key: &str) -> Vec<Command> {
        let Some(lease) = self.leases.get(key).copied() else {
            return Vec::new();
        };
        let orders = self.election.unregister(key, lease);
        let commands = self.commands_for(orders);
        self.leases.remove(key);
        self.keys_by_lease.remove(&lease);
        self.pending_wanted.remove(key);
        self.retire(key);
        commands
    }

    pub(super) fn next_generation(&mut self) -> u64 {
        self.loads += 1;
        self.loads
    }

    pub(super) fn retire(&mut self, key: &str) {
        let is_owned = self.sliced.contains(key)
            || self.loaded.contains_key(key)
            || self.leases.contains_key(key)
            || self.views.contains_key(key)
            || self.declarations.contains_key(key);
        if !is_owned {
            self.catalog.remove(key);
            self.pending_wanted.remove(key);
        }
    }

    pub(super) fn refresh_pending(&mut self, key: &str) {
        if self.leases.contains_key(key) && self.catalog.contains_key(key) && !self.loaded.contains_key(key) {
            self.pending_wanted.insert(key.to_string());
        } else {
            self.pending_wanted.remove(key);
        }
    }

    pub(super) fn has_pending_restore(&self) -> bool {
        self.ring.is_restoring() || self.ring.has_refused() || (!self.ring.is_background() && !self.pending_wanted.is_empty())
    }

    pub(super) fn next_due(&self) -> Option<u64> {
        let attempt = (self.is_restore_requested && self.has_pending_restore()).then_some(0);
        self.ring.next_due().into_iter().chain(attempt).min()
    }

    pub(super) fn restore_wanted(&mut self, now: u64) -> (Vec<Command>, bool) {
        let wanted: Vec<String> = self.pending_wanted.iter().cloned().collect();
        let mut commands = Vec::new();
        let mut is_refused = false;
        for key in wanted {
            let touched = self.ring.touch(now, &key);
            is_refused |= !touched.iter().any(|command| matches!(command, Command::Load { .. }));
            commands.extend(touched);
            if self.is_playing(&key) {
                commands.extend(self.ring.set_playing(&key, true));
            }
        }
        (commands, is_refused)
    }

    pub(super) fn playing_key(&self) -> Option<String> {
        self.keys_by_lease.get(&self.election.active()?).cloned()
    }

    pub(super) fn is_playing(&self, key: &str) -> bool {
        self.leases.get(key).is_some_and(|lease| self.election.active() == Some(*lease))
    }

    pub(super) fn playing_slots(&self) -> Vec<(usize, String, u64)> {
        self.slot_keys
            .iter()
            .filter(|(_, key)| self.is_playing(key))
            .filter_map(|(slot, key)| Some((*slot, key.clone(), self.loaded.get(key)?.generation)))
            .collect()
    }

    pub(super) fn is_loaded_as(&self, key: &str, slot: usize, generation: u64) -> bool {
        self.loaded.get(key).is_some_and(|loaded| loaded.slot == slot && loaded.generation == generation)
    }

    pub(super) fn playing_page(&self, keys: &impl PageKeys, pages: Range<usize>) -> Option<usize> {
        let lease = self.election.active()?;
        let playing = self.keys_by_lease.get(&lease)?;
        pages.into_iter().find(|page| keys.key(*page) == Some(playing.as_str()))
    }

    pub(super) fn commands_for(&mut self, orders: Vec<session::LeaseOrder>) -> Vec<Command> {
        let mut commands = Vec::new();
        for directive in orders {
            let Some(key) = self.keys_by_lease.get(&directive.lease).cloned() else {
                continue;
            };
            commands.extend(self.ring.set_playing(&key, directive.order == session::Order::Play));
        }
        commands
    }
}

pub(super) struct StageGuard<'a> {
    pub(super) guard: MutexGuard<'a, StageState>,
    pub(super) inner: &'a Inner,
    pub(super) is_changed: bool,
    pub(super) is_quiet: bool,
}

impl Deref for StageGuard<'_> {
    type Target = StageState;

    fn deref(&self) -> &StageState {
        &self.guard
    }
}

impl DerefMut for StageGuard<'_> {
    fn deref_mut(&mut self) -> &mut StageState {
        self.is_changed = true;
        &mut self.guard
    }
}

impl Drop for StageGuard<'_> {
    fn drop(&mut self) {
        if !self.is_changed {
            return;
        }
        self.inner.change.mark();
        if !self.is_quiet && self.guard.has_pending_restore() {
            self.guard.is_restore_requested = true;
        }
        let next = self.guard.next_due();
        self.inner.due.send_if_modified(|due| std::mem::replace(due, next) != next);
    }
}
