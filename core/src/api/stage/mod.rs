mod events;
mod inputs;
mod lifecycle;
mod playhead;
mod positions;

use self::positions::ResumePositions;
use super::places::Places;
use super::routes::Routes;
use super::slots::{SlotCall, SlotRunner};
use super::source::{MediaItem, PlaybackRoute};
use super::types::Kind;
use crate::cache::Clock;
use crate::change::Change;
use crate::hls::is_manifest_url;
use crate::memory::Ledger;
use crate::pages::PageKeys;
use crate::pool::{Command, Ring};
use crate::prefetch::Prefetcher;
use crate::session::{self, next_action, Action, Context, Election, Event, RecoveryState, WatchdogConfig};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::watch;
use super::guard::AbortOnDrop;

pub struct StageParts {
    pub ring: Ring,
    pub runner: Arc<SlotRunner>,
    pub routes: Arc<Routes>,
    pub places: Arc<Places>,
    pub prefetcher: Prefetcher,
    pub clock: Arc<dyn Clock>,
    pub watchdog: WatchdogConfig,
    pub silent: bool,
    pub change: Arc<Change>,
    pub ledger: Ledger,
}

struct Loaded {
    slot: usize,
    recovery: RecoveryState,
    source: MediaItem,
    route: PlaybackRoute,
}

struct State {
    ring: Ring,
    election: Election,
    leases: HashMap<String, u64>,
    keys_by_lease: HashMap<u64, String>,
    loaded: HashMap<String, Loaded>,
    slot_keys: HashMap<usize, String>,
    tasks: HashMap<String, AbortOnDrop>,
    catalog: HashMap<String, MediaItem>,
    views: HashMap<String, u64>,
    shown: HashMap<String, usize>,
    declarations: HashMap<String, Arc<()>>,
    rates: HashMap<String, f32>,
    positions: ResumePositions,
    window: Vec<Option<MediaItem>>,
    image_windows: HashMap<String, Vec<Vec<MediaItem>>>,
    is_restore_requested: bool,
    is_room_wanted: bool,
    refusal_room: u64,
    pending_wanted: HashSet<String>,
}

struct Inner {
    state: Mutex<State>,
    runner: Arc<SlotRunner>,
    routes: Arc<Routes>,
    places: Arc<Places>,
    prefetcher: Prefetcher,
    clock: Arc<dyn Clock>,
    watchdog: WatchdogConfig,
    silent: bool,
    handle: Handle,
    tick: Mutex<Option<AbortOnDrop>>,
    due: watch::Sender<Option<u64>>,
    change: Arc<Change>,
    foreground_passes: AtomicU64,
    wakes: AtomicU64,
    seeks: Mutex<HashMap<String, (u64, std::time::Instant)>>,
    hints: Mutex<HashMap<usize, u64>>,
    #[cfg(test)]
    declaration_completed: tokio::sync::Notify,
}

#[derive(Clone)]
pub struct Stage {
    inner: Arc<Inner>,
}

impl State {
    fn leave(&mut self, key: &str) -> bool {
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

    fn lease_for(&mut self, key: &str) -> u64 {
        if let Some(lease) = self.leases.get(key) {
            return *lease;
        }
        let lease = self.election.register(key, Context::Ring.policy());
        self.leases.insert(key.to_string(), lease);
        self.keys_by_lease.insert(lease, key.to_string());
        self.refresh_pending(key);
        lease
    }

    fn forget_lease(&mut self, key: &str) -> Vec<Command> {
        let Some(lease) = self.leases.get(key).copied() else {
            return Vec::new();
        };
        let orders = self.election.unregister(key, lease);
        let commands = self.commands_for(orders);
        self.leases.remove(key);
        self.keys_by_lease.remove(&lease);
        self.pending_wanted.remove(key);
        commands
    }

    fn refresh_pending(&mut self, key: &str) {
        if self.leases.contains_key(key) && self.catalog.contains_key(key) && !self.loaded.contains_key(key) {
            self.pending_wanted.insert(key.to_string());
        } else {
            self.pending_wanted.remove(key);
        }
    }

    fn prune_pending(&mut self) {
        let catalog = &self.catalog;
        self.pending_wanted.retain(|key| catalog.contains_key(key));
    }

    fn has_pending_restore(&self) -> bool {
        self.ring.is_restoring() || (!self.ring.is_background() && !self.pending_wanted.is_empty())
    }

    fn next_due(&self) -> Option<u64> {
        let attempt = (self.is_restore_requested && self.has_pending_restore()).then_some(0);
        self.ring.next_due().into_iter().chain(attempt).min()
    }

    fn restore_wanted(&mut self, now: u64) -> (Vec<Command>, bool) {
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

    fn is_playing(&self, key: &str) -> bool {
        self.leases.get(key).is_some_and(|lease| self.election.active() == Some(*lease))
    }

    fn remember_playing_positions(&mut self, runner: &SlotRunner) {
        let playing: Vec<(usize, String)> = self.slot_keys.iter().filter(|(_, key)| self.is_playing(key)).map(|(slot, key)| (*slot, key.clone())).collect();
        for (slot, key) in playing {
            if let Some(position) = runner.sample_now(slot).map(|sample| sample.position_ms).filter(|position| *position > 0) {
                self.positions.remember(&key, position);
            }
        }
    }

    fn playing_page(&self, keys: &impl PageKeys, pages: Range<usize>) -> Option<usize> {
        let lease = self.election.active()?;
        let playing = self.keys_by_lease.get(&lease)?;
        pages.into_iter().find(|page| keys.key(*page) == Some(playing.as_str()))
    }

    fn commands_for(&mut self, orders: Vec<session::LeaseOrder>) -> Vec<Command> {
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

impl Stage {
    pub fn new(handle: Handle, parts: StageParts) -> Stage {
        let state = State {
            ring: parts.ring,
            election: Election::new(),
            leases: HashMap::new(),
            keys_by_lease: HashMap::new(),
            loaded: HashMap::new(),
            slot_keys: HashMap::new(),
            tasks: HashMap::new(),
            catalog: HashMap::new(),
            views: HashMap::new(),
            shown: HashMap::new(),
            declarations: HashMap::new(),
            rates: HashMap::new(),
            positions: ResumePositions::default(),
            window: Vec::new(),
            image_windows: HashMap::new(),
            is_restore_requested: false,
            is_room_wanted: false,
            refusal_room: 0,
            pending_wanted: HashSet::new(),
        };
        let inner = Arc::new(Inner {
            state: Mutex::new(state),
            runner: parts.runner,
            routes: parts.routes,
            places: parts.places,
            prefetcher: parts.prefetcher,
            clock: parts.clock,
            watchdog: parts.watchdog,
            silent: parts.silent,
            handle: handle.clone(),
            tick: Mutex::new(None),
            due: watch::channel(None).0,
            change: parts.change,
            foreground_passes: AtomicU64::new(0),
            wakes: AtomicU64::new(0),
            seeks: Mutex::new(HashMap::new()),
            hints: Mutex::new(HashMap::new()),
            #[cfg(test)]
            declaration_completed: tokio::sync::Notify::new(),
        });
        let tick = handle.spawn(tick_loop(Arc::downgrade(&inner), inner.due.subscribe(), parts.ledger));
        *inner.tick.lock().unwrap_or_else(PoisonError::into_inner) = Some(AbortOnDrop::new(tick));
        let stage = Stage { inner };
        stage.inner.routes.set_playhead(stage.playhead());
        stage
    }

    pub fn wakes(&self) -> u64 {
        self.inner.wakes.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn commands_idle(&self) -> bool {
        self.inner.runner.is_idle()
    }

    #[cfg(test)]
    pub fn armed_tasks(&self) -> usize {
        self.state().tasks.len()
    }

    #[cfg(test)]
    pub fn foreground_passes(&self) -> u64 {
        self.inner.foreground_passes.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(test)]
    pub fn kept_preferences(&self) -> usize {
        let state = self.state();
        state.rates.len() + state.ring.muted_key_count()
    }

    pub fn slot_count(&self) -> usize {
        self.state().ring.slots()
    }

    fn forget_preferences(&self, key: &str) {
        self.forget_preferences_from(key, false);
    }

    fn forget_preferences_from(&self, key: &str, is_restore_attempt: bool) {
        if self.inner.places.is_declared(key) {
            return;
        }
        let mut state = if is_restore_attempt { self.state_quiet() } else { self.state() };
        if !state.loaded.contains_key(key) {
            state.rates.remove(key);
            state.ring.forget_mute(key);
        }
    }

    fn state(&self) -> StateGuard<'_> {
        StateGuard { guard: self.inner.state.lock().unwrap_or_else(PoisonError::into_inner), inner: &self.inner, is_changed: false, is_quiet: false }
    }

    fn state_quiet(&self) -> StateGuard<'_> {
        StateGuard { guard: self.inner.state.lock().unwrap_or_else(PoisonError::into_inner), inner: &self.inner, is_changed: false, is_quiet: true }
    }

    fn now(&self) -> u64 {
        self.inner.clock.now_ms()
    }

    fn dispatch(&self, mut state: StateGuard<'_>, commands: Vec<Command>) {
        if commands.is_empty() {
            return;
        }
        let mut calls = Vec::new();
        let mut actions = Vec::new();
        let mut unloaded = Vec::new();
        let mut resumed = Vec::new();
        for command in commands {
            crate::trace!("stage", "command {command:?}");
            self.translate(&mut state, command, &mut calls, &mut actions, &mut unloaded, &mut resumed);
        }
        for (slot, call) in calls {
            self.inner.runner.send(slot, call);
        }
        for key in unloaded {
            self.inner.places.unloaded(&key);
            if !self.inner.places.is_declared(&key) && !state.loaded.contains_key(&key) {
                state.rates.remove(&key);
                state.ring.forget_mute(&key);
            }
        }
        for key in resumed {
            self.inner.places.resumed(&key);
        }
        drop(state);
        self.refresh_hints();
        for (key, action) in actions {
            self.perform(key, action);
        }
    }

    fn translate(
        &self,
        state: &mut State,
        command: Command,
        calls: &mut Vec<(usize, SlotCall)>,
        actions: &mut Vec<(String, Action)>,
        unloaded: &mut Vec<String>,
        resumed: &mut Vec<String>,
    ) {
        match &command {
            Command::Load { slot, key } => {
                let source = state.catalog.get(key).cloned().unwrap_or_else(|| MediaItem::bare(key, Kind::Video));
                let route = self.inner.routes.play_url(&source);
                let via_proxy_hls = route.generation.is_some() && is_manifest_url(&source.url);
                let mut recovery = RecoveryState::new(self.inner.watchdog.clone());
                next_action(&mut recovery, Event::Source { key: key.clone(), via_proxy_hls }, self.now());
                state.slot_keys.insert(*slot, key.clone());
                state.loaded.insert(key.clone(), Loaded { slot: *slot, recovery, source, route: route.clone() });
                state.pending_wanted.remove(key);
                let resume = state.positions.take(key);
                calls.extend(self.load_calls(state, key, resume));
            }
            Command::Unload { slot } => {
                if let Some(key) = state.slot_keys.remove(slot) {
                    let position = self.inner.runner.sample_now(*slot).map(|sample| sample.position_ms).filter(|position| *position > 0);
                    if let Some(position) = position.filter(|_| !state.is_playing(&key)) {
                        state.positions.remember(&key, position);
                    }
                    state.loaded.remove(&key);
                    state.refresh_pending(&key);
                    state.tasks.remove(&key);
                    unloaded.push(key);
                }
                calls.extend(SlotCall::from_command(command, None));
            }
            Command::Play { slot } | Command::Pause { slot } => {
                let event = if matches!(command, Command::Play { .. }) { Event::Play { position_ms: 0 } } else { Event::Pause };
                if let Some(key) = state.slot_keys.get(slot).cloned() {
                    if matches!(command, Command::Play { .. }) {
                        if let Some(loaded) = state.loaded.get(&key) {
                            self.inner.routes.nudge_window(&loaded.source.url);
                        }
                        resumed.push(key.clone());
                    }
                    self.advance(state, &key, event, actions);
                }
                calls.extend(SlotCall::from_command(command, None));
            }
            Command::Unmute { slot } if self.inner.silent => calls.push((*slot, SlotCall::Mute)),
            _ => calls.extend(SlotCall::from_command(command, None)),
        }
    }

    fn load_calls(&self, state: &State, key: &str, resume: Option<u64>) -> Vec<(usize, SlotCall)> {
        let loaded = &state.loaded[key];
        let slot = loaded.slot;
        let mut calls = vec![(slot, SlotCall::Load { key: key.to_string(), route: loaded.route.clone() })];
        self.inner.hints.lock().unwrap_or_else(PoisonError::into_inner).remove(&slot);
        if self.inner.silent {
            calls.push((slot, SlotCall::Mute));
        }
        if let Some(rate) = state.rates.get(key) {
            calls.push((slot, SlotCall::Rate { rate: *rate }));
        }
        if let Some(position_ms) = resume {
            self.note_seek(&loaded.source.url, position_ms);
            calls.push((slot, SlotCall::Seek { position_ms }));
        }
        if let Some(view_token) = state.views.get(key) {
            calls.push((slot, SlotCall::Attach { view_token: *view_token }));
        }
        calls
    }

    fn advance(&self, state: &mut State, key: &str, event: Event, actions: &mut Vec<(String, Action)>) {
        let now = self.now();
        let Some(loaded) = state.loaded.get_mut(key) else { return };
        let action = next_action(&mut loaded.recovery, event, now);
        if action != Action::None {
            actions.push((key.to_string(), action));
        }
    }
}

async fn tick_loop(inner: Weak<Inner>, mut due: watch::Receiver<Option<u64>>, ledger: Ledger) {
    let freed = ledger.freed().notified();
    tokio::pin!(freed);
    freed.as_mut().enable();
    loop {
        let wait = match *due.borrow_and_update() {
            Some(at) => {
                let Some(inner) = inner.upgrade() else { return };
                Some(Duration::from_millis(at.saturating_sub(inner.clock.now_ms())))
            }
            None => None,
        };
        let wake = tokio::select! {
            () = async { tokio::time::sleep(wait.unwrap_or_default()).await }, if wait.is_some() => Wake::Due,
            changed = due.changed() => {
                if changed.is_err() {
                    return;
                }
                Wake::Changed
            }
            () = freed.as_mut() => Wake::Freed,
        };
        let Some(inner) = inner.upgrade() else { return };
        inner.wakes.fetch_add(1, Ordering::Relaxed);
        let stage = Stage { inner };
        match wake {
            Wake::Changed => continue,
            Wake::Freed => {
                freed.set(ledger.freed().notified());
                freed.as_mut().enable();
                let mut state = stage.state_quiet();
                let room = ledger.ceiling().saturating_sub(ledger.used());
                if state.is_room_wanted && room > state.refusal_room && state.has_pending_restore() {
                    state.is_restore_requested = true;
                }
                continue;
            }
            Wake::Due => {}
        }
        let (state, commands) = {
            let now = stage.now();
            let mut state = stage.state_quiet();
            let is_requested = std::mem::take(&mut state.is_restore_requested);
            let queued = state.ring.restore_len();
            let mut commands = state.ring.tick(now);
            let drained = queued.saturating_sub(state.ring.restore_len());
            let mut is_refused = state.ring.is_restoring() && drained == 0 && queued > 0;
            if is_requested {
                let (restored, refused) = state.restore_wanted(now);
                commands.extend(restored);
                is_refused |= refused;
            }
            state.is_restore_requested = state.ring.is_restoring() && drained > 0;
            if is_requested || queued > 0 {
                state.is_room_wanted = is_refused;
                state.refusal_room = ledger.ceiling().saturating_sub(ledger.used());
            }
            (state, commands)
        };
        stage.dispatch(state, commands);
    }
}

enum Wake {
    Due,
    Changed,
    Freed,
}

struct StateGuard<'a> {
    guard: MutexGuard<'a, State>,
    inner: &'a Inner,
    is_changed: bool,
    is_quiet: bool,
}

impl Deref for StateGuard<'_> {
    type Target = State;

    fn deref(&self) -> &State {
        &self.guard
    }
}

impl DerefMut for StateGuard<'_> {
    fn deref_mut(&mut self) -> &mut State {
        self.is_changed = true;
        &mut self.guard
    }
}

impl Drop for StateGuard<'_> {
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

#[cfg(test)]
mod events_tests;
#[cfg(test)]
mod inputs_tests;
#[cfg(test)]
mod playhead_tests;
#[cfg(test)]
mod strategy_tests;

#[cfg(test)]
mod positions_tests;
