mod events;
mod inputs;
mod lifecycle;
mod playhead;
mod positions;
mod dispatch;
mod state;

use self::positions::ResumePositions;
use super::places::Places;
use super::routes::Routes;
use super::slots::SlotRunner;
use crate::cache::Clock;
use crate::change::Change;
use crate::memory::Ledger;
use crate::pool::Ring;
use crate::prefetch::Prefetcher;
use crate::session::{Election, WatchdogConfig};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use tokio::runtime::Handle;
use tokio::sync::{watch, Notify};
use super::guard::AbortOnDrop;
use self::dispatch::tick_loop;
use self::state::{Armed, Declaration, LoadId, StageState, StageGuard, Work};

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

struct Inner {
    state: Mutex<StageState>,
    runner: Arc<SlotRunner>,
    routes: Arc<Routes>,
    places: Arc<Places>,
    prefetcher: Prefetcher,
    clock: Arc<dyn Clock>,
    watchdog: WatchdogConfig,
    silent: bool,
    handle: Handle,
    tick: Mutex<Option<AbortOnDrop>>,
    next_task: AtomicU64,
    due: watch::Sender<Option<u64>>,
    change: Arc<Change>,
    foreground_passes: AtomicU64,
    wakes: AtomicU64,
    seeks: Mutex<HashMap<String, (u64, std::time::Instant)>>,
    hints: Mutex<HashMap<usize, u64>>,
    playback_changes: Mutex<HashSet<String>>,
    playback_wake: Notify,
    playback_observed: AtomicBool,
    #[cfg(test)]
    action_completed: tokio::sync::Notify,
    #[cfg(test)]
    tick_wait: Mutex<Option<Option<u64>>>,
}

#[derive(Clone)]
pub struct Stage {
    inner: Arc<Inner>,
}

impl Stage {
    pub(crate) fn begin_pause_barrier(&self, sequence: u64, progress: tokio::sync::watch::Sender<(u64, std::collections::BTreeMap<u64, (u64, u64)>)>) {
        self.inner.runner.begin_pause_barrier(sequence, progress);
    }

    pub(crate) fn end_pause_barrier(&self) {
        self.inner.runner.end_pause_barrier();
    }

    pub fn new(handle: Handle, parts: StageParts) -> Stage {
        let state = StageState {
            ring: parts.ring,
            election: Election::new(),
            leases: HashMap::new(),
            keys_by_lease: HashMap::new(),
            loaded: HashMap::new(),
            slot_keys: HashMap::new(),
            tasks: HashMap::new(),
            foreground: None,
            catalog: HashMap::new(),
            sliced: HashSet::new(),
            views: HashMap::new(),
            shown: HashMap::new(),
            declarations: HashMap::new(),
            rates: HashMap::new(),
            positions: ResumePositions::default(),
            window: Arc::default(),
            image_windows: HashMap::new(),
            is_restore_requested: false,
            is_room_wanted: false,
            refusal_room: 0,
            pending_wanted: HashSet::new(),
            loads: 0,
            releases: 0,
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
            next_task: AtomicU64::new(0),
            due: watch::channel(None).0,
            change: parts.change,
            foreground_passes: AtomicU64::new(0),
            wakes: AtomicU64::new(0),
            seeks: Mutex::new(HashMap::new()),
            hints: Mutex::new(HashMap::new()),
            playback_changes: Mutex::new(HashSet::new()),
            playback_wake: Notify::new(),
            playback_observed: AtomicBool::new(false),
            #[cfg(test)]
            action_completed: tokio::sync::Notify::new(),
            #[cfg(test)]
            tick_wait: Mutex::new(None),
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
        self.state().tasks.values().map(Vec::len).sum()
    }

    #[cfg(test)]
    pub fn slot_records(&self) -> usize {
        self.inner.runner.live_slots()
    }

    #[cfg(test)]
    pub fn shown_count(&self, key: &str) -> usize {
        self.state().shown.get(key).copied().unwrap_or(0)
    }

    #[cfg(test)]
    pub fn holders(&self) -> usize {
        Arc::strong_count(&self.inner)
    }

    #[cfg(test)]
    pub fn repairs_in_flight(&self) -> usize {
        self.state().tasks.values().flatten().filter(|armed| matches!(armed.work, Work::Repair | Work::Purge)).count()
    }

    #[cfg(test)]
    pub fn has_foreground(&self) -> bool {
        self.state().foreground.is_some()
    }

    #[cfg(test)]
    pub fn declaration_checks(&self) -> usize {
        self.state().declarations.values().filter(|declaration| declaration.check.is_some()).count()
    }

    #[cfg(test)]
    pub fn armed_generation(&self, key: &str) -> Option<u64> {
        self.state().tasks.get(key).and_then(|armed| armed.iter().find(|armed| armed.work.is_timer())).map(|armed| armed.generation)
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

    fn next_token(&self) -> u64 {
        self.inner.next_task.fetch_add(1, Ordering::Relaxed)
    }

    fn track(&self, id: &LoadId, work: Work, token: u64, body: impl std::future::Future<Output = ()> + Send + 'static) {
        self.with_id(id, |state| {
            let task = AbortOnDrop::new(self.inner.handle.spawn(body));
            let entries = state.tasks.entry(id.key.clone()).or_default();
            if work.is_timer() {
                entries.retain(|armed| armed.work != work);
            }
            entries.push(Armed { generation: id.generation, work, token, _task: task });
        });
    }

    fn finish_work(&self, key: &str, token: u64) {
        let finished = {
            let mut state = self.inner.state.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(entries) = state.tasks.get_mut(key) else { return };
            let position = entries.iter().position(|armed| armed.token == token);
            let finished = position.map(|position| entries.remove(position));
            if entries.is_empty() {
                state.tasks.remove(key);
            }
            finished
        };
        drop(finished);
    }

    pub async fn close(&self) {
        self.inner.tick.lock().unwrap_or_else(PoisonError::into_inner).take();
        self.inner.prefetcher.stop_all();
        self.inner.runner.dispose_all();
        let released = {
            let mut state = self.inner.state.lock().unwrap_or_else(PoisonError::into_inner);
            let released = (std::mem::take(&mut state.tasks), state.foreground.take(), std::mem::take(&mut state.declarations), state.ring.close());
            state.loaded.clear();
            state.slot_keys.clear();
            released
        };
        drop(released);
        self.inner.runner.disposed().await;
    }

    fn state(&self) -> StageGuard<'_> {
        let guard = self.inner.state.lock().unwrap_or_else(PoisonError::into_inner);
        let playing = self.inner.playback_observed.load(Ordering::Acquire).then(|| guard.reported_playing_key()).flatten();
        StageGuard { guard, inner: &self.inner, is_changed: false, is_quiet: false, playing }
    }

    fn state_quiet(&self) -> StageGuard<'_> {
        let guard = self.inner.state.lock().unwrap_or_else(PoisonError::into_inner);
        let playing = self.inner.playback_observed.load(Ordering::Acquire).then(|| guard.reported_playing_key()).flatten();
        StageGuard { guard, inner: &self.inner, is_changed: false, is_quiet: true, playing }
    }

    pub(crate) fn playback_wake(&self) -> &Notify { &self.inner.playback_wake }

    pub(crate) fn observe_playback(&self) { self.inner.playback_observed.store(true, Ordering::Release); }

    pub(crate) fn take_playback_changes(&self) -> HashSet<String> {
        std::mem::take(&mut *self.inner.playback_changes.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn now(&self) -> u64 {
        self.inner.clock.now_ms()
    }
}

#[cfg(test)]
mod events_tests;
#[cfg(test)]
mod inputs_move_tests;
#[cfg(test)]
mod inputs_place_tests;
#[cfg(test)]
mod inputs_suspension_tests;
#[cfg(test)]
mod inputs_window_tests;
#[cfg(test)]
mod lifetime_tests;
#[cfg(test)]
mod playhead_tests;
#[cfg(test)]
mod reservation_tests;
#[cfg(test)]
mod tasks_tests;
#[cfg(test)]
mod strategy_tests;

#[cfg(test)]
mod positions_tests;
