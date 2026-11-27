use crate::api::slots::SlotCall;
use crate::api::source::MediaItem;
use crate::api::types::Kind;
use crate::hls::is_manifest_url;
use crate::memory::Ledger;
use crate::pool::Command;
use crate::session::{next_action, Action, Event, RecoveryState};
use std::sync::atomic::Ordering;
use std::sync::{PoisonError, Weak};
use std::time::Duration;
use tokio::sync::watch;
use super::{Inner, Stage};
use super::state::*;

impl Stage {
    pub(super) fn dispatch(&self, state: StageGuard<'_>, commands: Vec<Command>) {
        self.dispatch_with(state, commands, true, Vec::new());
    }

    pub(super) fn dispatch_with(&self, mut state: StageGuard<'_>, commands: Vec<Command>, is_remembering: bool, follow: Vec<(usize, SlotCall)>) {
        if commands.is_empty() && follow.is_empty() {
            return;
        }
        let (disposals, mut commands): (Vec<Command>, Vec<Command>) = commands.into_iter().partition(|command| matches!(command, Command::Dispose { .. }));
        let loads: Vec<(usize, String)> = commands
            .iter()
            .filter_map(|command| if let Command::Load { slot, key, .. } = command { Some((*slot, key.clone())) } else { None })
            .collect();
        for (slot, key) in &loads {
            if state.is_playing(key) && !commands.iter().any(|command| matches!(command, Command::Play { slot: playing } if playing == slot)) {
                commands.extend(state.ring.set_playing(key, true));
            }
        }
        let mut effects = Effects::default();
        for command in commands {
            crate::trace!("stage", "command {command:?}");
            if let Command::Play { slot } = &command {
                if !loads.iter().any(|(loaded_slot, _)| loaded_slot == slot) {
                    if let Some(key) = state.slot_keys.get(slot) {
                        if let Some(loaded) = state.loaded.get(key).filter(|loaded| loaded.failed || loaded.ready_generation != Some(loaded.generation)) {
                            effects.reload_on_play.push(LoadId { key: key.clone(), slot: *slot, generation: loaded.generation });
                            continue;
                        }
                    }
                }
            }
            self.translate(&mut state, command, &mut effects);
        }
        let Effects { calls, actions, unloaded, resumed, parked, reload_on_play } = effects;
        for (slot, call) in calls.into_iter().chain(follow) {
            self.send_call(&state, slot, call);
        }
        for command in disposals {
            if let Command::Dispose { slot } = command {
                self.inner.runner.dispose(slot);
            }
        }
        for key in unloaded {
            self.inner.places.unloaded(&key);
            if !self.inner.places.is_declared(&key) && !state.loaded.contains_key(&key) {
                state.rates.remove(&key);
                state.ring.forget_mute(&key);
            }
        }
        for (_, key, _) in &resumed {
            self.inner.places.resumed(key);
        }
        drop(state);
        for id in reload_on_play {
            crate::trace!("heal", "slot reload-on-play key={}", crate::trace::tail(&id.key));
            self.reload(&id, true);
        }
        if is_remembering {
            self.remember_parked(parked);
        }
        for (slot, key, generation) in resumed {
            self.apply_pending_fraction(slot, &key, generation, true);
        }
        self.refresh_hints();
        for (id, action) in actions {
            self.perform(id, action);
        }
    }

    pub(super) fn send_call(&self, state: &StageState, slot: usize, call: SlotCall) {
        let held = match &call {
            SlotCall::Load { key, .. } => state.loaded.get(key).map(|loaded| loaded.held.clone()),
            _ => None,
        };
        self.inner.runner.send_held(slot, call, held);
        #[cfg(test)]
        crate::test_signal::fire();
    }

    pub(super) fn translate(
        &self,
        state: &mut StageState,
        command: Command,
        effects: &mut Effects,
    ) {
        let Effects { calls, actions, unloaded, resumed, parked, .. } = effects;
        match &command {
            Command::Load { slot, key, held } => {
                let source = state.catalog.get(key).cloned().unwrap_or_else(|| MediaItem::bare(key, Kind::Video));
                let route = self.inner.routes.play_url(&source);
                let via_proxy_hls = route.generation.is_some() && is_manifest_url(&source.url);
                let mut recovery = RecoveryState::new(self.inner.watchdog.clone());
                next_action(&mut recovery, Event::Source { key: key.clone(), via_proxy_hls }, self.now());
                state.slot_keys.insert(*slot, key.clone());
                let generation = state.next_generation();
                state.loaded.insert(key.clone(), Loaded { held: held.clone(), slot: *slot, generation, recovery, source, route: route.clone(), seeked: None, failed: false, ready_generation: None });
                state.pending_wanted.remove(key);
                let resume = state.positions.take(key);
                calls.extend(self.load_calls(state, key, resume));
            }
            Command::Unload { slot } => {
                if let Some(key) = state.slot_keys.remove(slot) {
                    if !state.is_playing(&key) {
                        let seeked = state.loaded.get(&key).and_then(|loaded| loaded.seeked);
                        parked.push(Parked { slot: *slot, key: key.clone(), releases: state.releases, seeked });
                    }
                    if let Some(unloaded) = state.loaded.remove(&key) {
                        self.forget_seek_hint(state, &unloaded.source.url);
                    }
                    state.retire(&key);
                    state.refresh_pending(&key);
                    state.tasks.remove(&key);
                    unloaded.push(key);
                }
                calls.extend(SlotCall::from_command(command));
            }
            Command::Dispose { .. } => {}
            Command::Play { slot } | Command::Pause { slot } => {
                let event = if matches!(command, Command::Play { .. }) { Event::Play { position_ms: 0 } } else { Event::Pause };
                if let Some(key) = state.slot_keys.get(slot).cloned() {
                    if let (Command::Play { .. }, Some(loaded)) = (&command, state.loaded.get_mut(&key)) {
                        loaded.seeked = None;
                        loaded.failed = false;
                        self.inner.routes.nudge_window(&loaded.source.url);
                        resumed.push((*slot, key.clone(), loaded.generation));
                    }
                    self.advance(state, &key, event, actions);
                }
                calls.extend(SlotCall::from_command(command.clone()));
            }
            Command::Unmute { slot } if self.inner.silent => calls.push((*slot, SlotCall::Mute)),
            _ => calls.extend(SlotCall::from_command(command)),
        }
    }

    pub(super) fn fraction_seek(&self, state: &mut StageState, key: &str, duration_ms: Option<u64>) -> Option<SlotCall> {
        let duration = duration_ms.filter(|duration| *duration > 0)?;
        let loaded = state.loaded.get(key)?;
        let fraction = state.positions.take_fraction(key)?;
        let position_ms = (duration as f64 * fraction) as u64;
        self.note_seek(&loaded.source.url, position_ms);
        Some(SlotCall::Seek { position_ms })
    }

    pub(super) fn remember_parked(&self, parked: Vec<Parked>) {
        let positions: Vec<(Parked, u64)> = parked
            .into_iter()
            .filter_map(|parked| {
                let position = match parked.seeked {
                    Some(seeked) => seeked,
                    None => self.inner.runner.sample_now(parked.slot)?.position_ms,
                };
                Some((parked, position))
            })
            .collect();
        if positions.is_empty() {
            return;
        }
        let mut state = self.state_quiet();
        for (Parked { slot, key, releases, .. }, position) in positions {
            if state.releases == releases && !state.slot_keys.contains_key(&slot) && !state.loaded.contains_key(&key) {
                if position > 0 {
                    state.positions.remember(&key, position);
                } else {
                    state.positions.forget(&key);
                }
            }
        }
    }

    pub(super) fn load_calls(&self, state: &StageState, key: &str, resume: Option<u64>) -> Vec<(usize, SlotCall)> {
        let loaded = &state.loaded[key];
        let slot = loaded.slot;
        let mut calls = vec![(slot, SlotCall::Load { key: key.to_string(), route: loaded.route.clone(), generation: loaded.generation })];
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

    pub(super) fn advance(&self, state: &mut StageState, key: &str, event: Event, actions: &mut Vec<(LoadId, Action)>) {
        let now = self.now();
        let Some(loaded) = state.loaded.get_mut(key) else { return };
        let action = next_action(&mut loaded.recovery, event, now);
        if action != Action::None {
            actions.push((LoadId { key: key.to_string(), slot: loaded.slot, generation: loaded.generation }, action));
        }
    }
}

pub(super) async fn tick_loop(inner: Weak<Inner>, mut due: watch::Receiver<Option<u64>>, ledger: Ledger) {
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
        #[cfg(test)]
        if let Some(inner) = inner.upgrade() {
            if let Ok(mut wait) = inner.tick_wait.lock() {
                *wait = Some(*due.borrow());
            }
            crate::test_signal::fire();
        }
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
            let room = ledger.ceiling().saturating_sub(ledger.used());
            let is_requested = std::mem::take(&mut state.is_restore_requested);
            let queued = state.ring.restore_len();
            let mut commands = state.ring.tick(now);
            let drained = queued.saturating_sub(state.ring.restore_len());
            let mut is_refused = state.ring.is_restoring() && drained == 0 && queued > 0;
            if is_requested {
                let (restored, refused) = state.restore_wanted(now);
                commands.extend(restored);
                is_refused |= refused;
                let playing = state.playing_key();
                commands.extend(state.ring.retry_refused(now, playing.as_deref()));
                is_refused |= state.ring.has_refused();
            }
            state.is_restore_requested = state.ring.is_restoring() && drained > 0;
            if is_requested || queued > 0 {
                state.is_room_wanted = is_refused;
                state.refusal_room = room;
            }
            (state, commands)
        };
        stage.dispatch(state, commands);
    }
}

pub(super) enum Wake {
    Due,
    Changed,
    Freed,
}
