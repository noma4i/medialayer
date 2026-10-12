mod events;
mod inputs;
mod lifecycle;
mod positions;

use self::positions::ResumePositions;
use super::places::Places;
use super::routes::Routes;
use super::slots::{SlotCall, SlotRunner};
use super::source::{MediaItem, PlaybackRoute};
use super::types::Kind;
use crate::cache::Clock;
use crate::hls::is_manifest_url;
use crate::pages::PageKeys;
use crate::pool::{Command, Ring};
use crate::prefetch::Prefetcher;
use crate::session::{self, next_action, Action, Context, Election, Event, RecoveryState, WatchdogConfig};
use std::collections::HashMap;
use std::ops::Range;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;
use tokio::runtime::Handle;
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
    pub tick: Duration,
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
    rates: HashMap<String, f32>,
    positions: ResumePositions,
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
    foreground_passes: AtomicU64,
}

#[derive(Clone)]
pub struct Stage {
    inner: Arc<Inner>,
}

impl State {
    fn lease_for(&mut self, key: &str) -> u64 {
        if let Some(lease) = self.leases.get(key) {
            return *lease;
        }
        let lease = self.election.register(key, Context::Ring.policy());
        self.leases.insert(key.to_string(), lease);
        self.keys_by_lease.insert(lease, key.to_string());
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
        commands
    }

    fn restore_wanted(&mut self, now: u64) -> Vec<Command> {
        let wanted: Vec<String> =
            self.leases.keys().filter(|key| self.catalog.contains_key(*key) && !self.loaded.contains_key(*key)).cloned().collect();
        let mut commands = Vec::new();
        for key in wanted {
            commands.extend(self.ring.touch(now, &key));
            if self.is_playing(&key) {
                commands.extend(self.ring.set_playing(&key, true));
            }
        }
        commands
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
            rates: HashMap::new(),
            positions: ResumePositions::default(),
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
            foreground_passes: AtomicU64::new(0),
        });
        let tick = handle.spawn(tick_loop(Arc::downgrade(&inner), parts.tick));
        *inner.tick.lock().unwrap_or_else(PoisonError::into_inner) = Some(AbortOnDrop::new(tick));
        Stage { inner }
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
        if self.inner.places.is_declared(key) {
            return;
        }
        let mut state = self.state();
        if !state.loaded.contains_key(key) {
            state.rates.remove(key);
            state.ring.forget_mute(key);
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn now(&self) -> u64 {
        self.inner.clock.now_ms()
    }

    fn dispatch(&self, commands: Vec<Command>) {
        if commands.is_empty() {
            return;
        }
        let mut calls = Vec::new();
        let mut actions = Vec::new();
        let mut unloaded = Vec::new();
        let mut resumed = Vec::new();
        {
            let mut state = self.state();
            for command in commands {
                crate::trace!("stage", "command {command:?}");
                self.translate(&mut state, command, &mut calls, &mut actions, &mut unloaded, &mut resumed);
            }
        }
        for (slot, call) in calls {
            self.inner.runner.send(slot, call);
        }
        for key in unloaded {
            self.inner.places.unloaded(&key);
            self.forget_preferences(&key);
        }
        for key in resumed {
            self.inner.places.resumed(&key);
        }
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
                let view = state.views.get(key).copied();
                let rate = state.rates.get(key).copied();
                let resume = state.positions.take(key);
                let slot = *slot;
                calls.push(SlotCall::from_command(command, Some(route)));
                if self.inner.silent {
                    calls.push((slot, SlotCall::Mute));
                }
                if let Some(rate) = rate {
                    calls.push((slot, SlotCall::Rate { rate }));
                }
                if let Some(position_ms) = resume {
                    calls.push((slot, SlotCall::Seek { position_ms }));
                }
                if let Some(view_token) = view {
                    calls.push((slot, SlotCall::Attach { view_token }));
                }
            }
            Command::Unload { slot } => {
                if let Some(key) = state.slot_keys.remove(slot) {
                    let position = self.inner.runner.sample_now(*slot).map(|sample| sample.position_ms).filter(|position| *position > 0);
                    if let Some(position) = position.filter(|_| !state.is_playing(&key)) {
                        state.positions.remember(&key, position);
                    }
                    state.loaded.remove(&key);
                    state.tasks.remove(&key);
                    unloaded.push(key);
                }
                calls.push(SlotCall::from_command(command, None));
            }
            Command::Play { slot } | Command::Pause { slot } => {
                let event = if matches!(command, Command::Play { .. }) { Event::Play { position_ms: 0 } } else { Event::Pause };
                if let Some(key) = state.slot_keys.get(slot).cloned() {
                    if matches!(command, Command::Play { .. }) {
                        resumed.push(key.clone());
                    }
                    self.advance(state, &key, event, actions);
                }
                calls.push(SlotCall::from_command(command, None));
            }
            Command::Unmute { slot } if self.inner.silent => calls.push((*slot, SlotCall::Mute)),
            _ => calls.push(SlotCall::from_command(command, None)),
        }
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

async fn tick_loop(inner: Weak<Inner>, every: Duration) {
    loop {
        tokio::time::sleep(every).await;
        let Some(inner) = inner.upgrade() else { return };
        let stage = Stage { inner };
        let commands = {
            let now = stage.now();
            let mut state = stage.state();
            let mut commands = state.ring.tick(now);
            commands.extend(state.restore_wanted(now));
            commands
        };
        stage.dispatch(commands);
    }
}

#[cfg(test)]
mod events_tests;
#[cfg(test)]
mod inputs_tests;

#[cfg(test)]
mod positions_tests;
