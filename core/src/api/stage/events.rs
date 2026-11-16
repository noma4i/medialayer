use super::{LoadId, Stage, StageGuard, Work};
use crate::api::slots::{SlotCall, SlotSample};
use crate::api::types::MediaState;
use crate::proxy::Readiness;
use crate::session::{Action, Event, Intent, Sample};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::PoisonError;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Wake {
    Watchdog,
    Retry,
    Foreground,
}

impl Stage {
    pub fn retry(&self, item: &crate::api::MediaItem) {
        self.inner.prefetcher.retry(&item.for_prefetch());
        let id = {
            let mut state = self.state();
            state.tasks.remove(&item.url);
            state.loaded.get_mut(&item.url).map(|loaded| {
                loaded.recovery = crate::session::RecoveryState::new(self.inner.watchdog.clone());
                crate::session::next_action(&mut loaded.recovery, Event::Source { key: item.url.clone(), via_proxy_hls: loaded.route.generation.is_some() && crate::hls::is_manifest_url(&item.url) }, self.now());
                loaded.failed = false;
                LoadId { key: item.url.clone(), slot: loaded.slot, generation: loaded.generation }
            })
        };
        if let Some(id) = id { self.reload(&id, true); }
        else if item.url.is_empty() { self.refuse_source(&item.url); }
        else if item.kind != crate::api::Kind::Image {
            let mut state = self.state();
            let commands = state.ring.touch(self.now(), &item.url);
            self.dispatch(state, commands);
        }
    }
    pub fn ready(&self, slot: usize, generation: u64) {
        crate::trace!("event", "ready slot={slot} generation={generation}");
        let Some(key) = self.feed_current(slot, generation, Event::Loaded) else { return };
        self.apply_pending_fraction(slot, &key, generation, false);
        self.with_current(slot, generation, |key| self.inner.places.ready(key));
        self.sync_foreground();
    }

    #[cfg(test)]
    pub(super) fn load_id_at(&self, slot: usize) -> Option<LoadId> {
        let state = self.state();
        let key = state.slot_keys.get(&slot)?;
        state.loaded.get(key).map(|loaded| LoadId { key: key.clone(), slot, generation: loaded.generation })
    }

    #[cfg(test)]
    pub fn reload_key(&self, key: &str) {
        let id = {
            let state = self.state();
            state.loaded.get(key).map(|loaded| LoadId { key: key.to_string(), slot: loaded.slot, generation: loaded.generation })
        };
        if let Some(id) = id {
            self.reload(&id, false);
        }
    }

    #[cfg(test)]
    pub fn generation_at(&self, slot: usize) -> u64 {
        let state = self.state();
        state.slot_keys.get(&slot).and_then(|key| state.loaded.get(key)).map_or(0, |loaded| loaded.generation)
    }

    fn current_key(state: &super::StageState, slot: usize, generation: u64) -> Option<String> {
        let key = state.slot_keys.get(&slot)?;
        state.is_loaded_as(key, slot, generation).then(|| key.clone())
    }

    fn with_current<R>(&self, slot: usize, generation: u64, work: impl FnOnce(&str) -> R) -> Option<R> {
        let state = self.state();
        let key = Self::current_key(&state, slot, generation)?;
        Some(work(&key))
    }

    fn feed_current(&self, slot: usize, generation: u64, event: Event) -> Option<String> {
        let mut actions = Vec::new();
        let key = {
            let mut state = self.state();
            let key = Self::current_key(&state, slot, generation)?;
            self.advance(&mut state, &key, event, &mut actions);
            key
        };
        for (id, action) in actions {
            self.perform(id, action);
        }
        Some(key)
    }

    pub(super) fn apply_pending_fraction(&self, slot: usize, key: &str, generation: u64, is_quiet: bool) {
        let duration = self.inner.runner.sample_now(slot).and_then(|sample| sample.duration_ms);
        let mut state = if is_quiet { self.state_quiet() } else { self.state() };
        if !state.is_loaded_as(key, slot, generation) {
            return;
        }
        if let Some(call) = self.fraction_seek(&mut state, key, duration) {
            self.inner.runner.send(slot, call);
        }
    }

    pub fn visual_ready(&self, slot: usize, generation: u64) {
        crate::trace!("event", "visual_ready slot={slot} generation={generation}");
        let key = self.with_current(slot, generation, str::to_string);
        if let Some(key) = key {
            self.apply_pending_fraction(slot, &key, generation, false);
            self.with_current(slot, generation, |key| self.inner.places.rendered(key));
        }
        self.sync_foreground();
    }

    pub fn stalled(&self, slot: usize, generation: u64, position_ms: u64) {
        crate::trace!("event", "stalled slot={slot} generation={generation} position={position_ms}");
        let url = {
            let state = self.state();
            Self::current_key(&state, slot, generation).filter(|key| state.is_playing(key)).and_then(|key| state.loaded.get(&key).map(|loaded| loaded.source.url.clone()))
        };
        let Some(url) = url else { return };
        self.inner.routes.grow_window(&url);
        self.feed_current(slot, generation, Event::Play { position_ms });
    }

    pub fn failed(&self, slot: usize, generation: u64, status: i32, detail: String) {
        crate::trace!("event", "failed slot={slot} generation={generation} status={status} detail={detail}");
        {
            let mut state = self.state();
            let Some(key) = Self::current_key(&state, slot, generation) else { return };
            if let Some(loaded) = state.loaded.get_mut(&key) { loaded.failed = true; }
        }
        let event = if status > 0 { Event::HttpError { status } } else { Event::PlaybackFailed { reason: detail } };
        if self.feed_current(slot, generation, event).is_some() {
            self.sync_foreground();
        }
    }

    pub fn ended(&self, slot: usize, generation: u64) {
        crate::trace!("event", "ended slot={slot} generation={generation}");
        let mut actions = Vec::new();
        {
            let mut state = self.state();
            let Some(key) = Self::current_key(&state, slot, generation) else {
                crate::trace!("event", "ended dropped: slot={slot} generation={generation} is not the current load");
                return;
            };
            let looping = state.loaded.get(&key).is_some_and(|loaded| loaded.route.looping);
            if looping || !self.inner.places.ended(&key) {
                crate::trace!("event", "ended dropped: {} looping={looping}", crate::trace::tail(&key));
                return;
            }
            self.advance(&mut state, &key, Event::Pause, &mut actions);
            state.positions.forget(&key);
            if let Some(loaded) = state.loaded.get_mut(&key) {
                loaded.seeked = Some(0);
                self.inner.seeks.lock().unwrap_or_else(PoisonError::into_inner).remove(&loaded.source.url);
            }
            let orders = match state.leases.get(&key).copied() {
                Some(lease) => state.election.set_intent(&key, lease, Intent::Idle),
                None => Vec::new(),
            };
            let commands = state.commands_for(orders);
            self.dispatch_with(state, commands, true, vec![(slot, SlotCall::Seek { position_ms: 0 })]);
        }
        for (id, action) in actions {
            self.perform(id, action);
        }
    }

    pub(super) fn sync_foreground(&self) {
        let playing = {
            let state = self.state();
            state.election.active().and_then(|lease| state.keys_by_lease.get(&lease).cloned())
        };
        let busy = playing.is_some_and(|key| self.inner.places.state(&key) == MediaState::Loading);
        self.inner.prefetcher.set_foreground_busy(busy);
    }

    pub(super) fn feed_id(&self, id: &LoadId, event: Event) -> bool {
        let mut actions = Vec::new();
        if self.with_id(id, |state| self.advance(state, &id.key, event, &mut actions)).is_none() {
            return false;
        }
        for (id, action) in actions {
            self.perform(id, action);
        }
        true
    }

    pub(super) fn with_id<R>(&self, id: &LoadId, work: impl FnOnce(&mut StageGuard<'_>) -> R) -> Option<R> {
        let mut state = self.state();
        if !state.is_loaded_as(&id.key, id.slot, id.generation) {
            return None;
        }
        Some(work(&mut state))
    }

    pub(super) fn perform(&self, id: LoadId, action: Action) {
        crate::trace!("event", "action {} {action:?}", crate::trace::tail(&id.key));
        match action {
            Action::None => {}
            Action::ForegroundRetry(delay_ms) => self.wake_after(id, delay_ms, Wake::Foreground),
            Action::Arm { delay_ms } => self.wake_after(id, delay_ms, Wake::Watchdog),
            Action::Retry(delay_ms) => self.wake_after(id, delay_ms, Wake::Retry),
            Action::Disarm => {
                self.with_id(&id, |state| {
                    if let Some(entries) = state.tasks.get_mut(&id.key) {
                        entries.retain(|armed| armed.work != Work::Watchdog);
                    }
                });
            }
            Action::Replay => {
                self.reload(&id, false);
            }
            Action::Recover => {
                let token = self.next_token();
                let (inner, routes, target) = (Arc::downgrade(&self.inner), self.inner.routes.clone(), id.clone());
                self.track(&id, Work::Repair, token, async move {
                    routes.repair().await;
                    let Some(inner) = inner.upgrade() else { return };
                    let stage = Stage { inner };
                    stage.reload(&target, true);
                    stage.finish_work(&target.key, token);
                    #[cfg(test)]
                    stage.inner.action_completed.notify_one();
                });
            }
            Action::PurgeCache(url) => {
                let token = self.next_token();
                let (inner, routes, target) = (Arc::downgrade(&self.inner), self.inner.routes.clone(), id.clone());
                self.track(&id, Work::Purge, token, async move {
                    routes.purge(&url).await;
                    let Some(inner) = inner.upgrade() else { return };
                    let stage = Stage { inner };
                    stage.feed_id(&target, Event::CachePurged);
                    stage.finish_work(&target.key, token);
                    #[cfg(test)]
                    stage.inner.action_completed.notify_one();
                });
            }
            Action::RefreshRoute => self.refresh_route(&id),
            Action::Fail(reason) => {
                let mut state = self.state();
                if !state.is_loaded_as(&id.key, id.slot, id.generation) { return; }
                self.inner.places.failed(&id.key, &reason);
                let orders = state.leases.get(&id.key).copied().map(|lease| state.election.set_intent(&id.key, lease, Intent::Idle)).unwrap_or_default();
                let commands = state.commands_for(orders);
                self.dispatch(state, commands);
            }
            Action::ForegroundPass => {
                if let Some((is_ok, current)) = self.foreground_pass(&id) {
                    self.feed_id(&current, Event::ForegroundPassDone { is_ok });
                }
            }
        }
    }

    fn wake_after(&self, id: LoadId, delay_ms: u64, wake: Wake) {
        let work = match wake {
            Wake::Watchdog => Work::Watchdog,
            Wake::Retry => Work::Retry,
            Wake::Foreground => Work::Foreground,
        };
        let token = self.next_token();
        let (inner, routes, runner, target) = (Arc::downgrade(&self.inner), self.inner.routes.clone(), self.inner.runner.clone(), id.clone());
        self.track(&id, work, token, async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            if wake != Wake::Watchdog {
                routes.repair().await;
            }
            let sample = if wake == Wake::Watchdog {
                let is_current = inner.upgrade().and_then(|inner| Stage { inner }.with_id(&target, |_| ()));
                if is_current.is_none() {
                    return;
                }
                let Some(sample) = runner.sample(target.slot).await else { return };
                Some(sample)
            } else {
                None
            };
            let Some(inner) = inner.upgrade() else { return };
            let stage = Stage { inner };
            match (wake, sample) {
                (Wake::Watchdog, Some(SlotSample { position_ms, duration_ms, is_looping, view_attached, frame_rendered })) => {
                    let sample = Sample { position_ms, duration_ms, is_looping, view_attached, frame_rendered };
                    stage.feed_id(&target, Event::WatchdogFired(sample));
                }
                (Wake::Retry, _) => stage.retry_due(&target),
                (Wake::Foreground, _) => {
                    stage.feed_id(&target, Event::ForegroundDue);
                }
                (Wake::Watchdog, None) => {}
            }
            stage.finish_work(&target.key, token);
        });
    }

    fn retry_due(&self, id: &LoadId) {
        let Some(resolved_generation) = self.with_id(id, |state| state.loaded.get(&id.key).map(|loaded| loaded.route.generation)).flatten() else {
            return;
        };
        let event = Event::RetryDue {
            resolved_generation,
            current_generation: self.inner.routes.generation(),
            readiness: self.inner.routes.readiness(),
        };
        self.feed_id(id, event);
    }

    fn refresh_route(&self, id: &LoadId) {
        let mut actions = Vec::new();
        let done = self.with_id(id, |state| {
            if let Some(loaded) = state.loaded.get_mut(&id.key) {
                loaded.route = self.inner.routes.play_url(&loaded.source);
            }
            self.advance(state, &id.key, Event::RouteRefreshed, &mut actions);
        });
        if done.is_some() {
            for (id, action) in actions {
                self.perform(id, action);
            }
        }
    }

    pub(super) fn reload(&self, id: &LoadId, is_refreshing: bool) -> Option<LoadId> {
        let (state, calls, current) = {
            let mut state = self.state();
            if !state.is_loaded_as(&id.key, id.slot, id.generation) {
                return None;
            }
            let key = id.key.as_str();
            if is_refreshing {
                if let Some(loaded) = state.loaded.get_mut(key) {
                    loaded.route = self.inner.routes.refresh(&loaded.source, &loaded.route);
                }
            }
            let generation = state.next_generation();
            let is_playing = state.is_playing(key);
            if let Some(loaded) = state.loaded.get_mut(key) {
                loaded.generation = generation;
                if is_playing { loaded.failed = false; }
            }
            if let Some(entries) = state.tasks.get_mut(key) {
                entries.retain(|armed| !armed.work.is_timer() || armed.generation == generation);
            }
            let mut calls = vec![(id.slot, SlotCall::Unload)];
            let resume = state.positions.take(key);
            calls.extend(self.load_calls(&state, key, resume));
            if is_playing {
                calls.push((id.slot, SlotCall::Play));
            }
            (state, calls, LoadId { key: id.key.clone(), slot: id.slot, generation })
        };
        let is_playing = calls.iter().any(|(_, call)| *call == SlotCall::Play);
        for (slot, call) in calls {
            self.send_call(&state, slot, call);
        }
        drop(state);
        self.refresh_hints();
        if is_playing {
            self.feed_id(&current, Event::Play { position_ms: 0 });
        }
        Some(current)
    }

    fn foreground_pass(&self, id: &LoadId) -> Option<(bool, LoadId)> {
        self.with_id(id, |_| ())?;
        self.inner.foreground_passes.fetch_add(1, Ordering::SeqCst);
        #[cfg(test)]
        crate::test_signal::fire();
        if self.inner.routes.readiness() != Readiness::Ready {
            return Some((false, id.clone()));
        }
        let is_stale = self.with_id(id, |state| state.loaded.get(&id.key).is_some_and(|loaded| self.inner.routes.is_stale(&loaded.route)))?;
        if !is_stale {
            self.replay_play(id);
            return Some((true, id.clone()));
        }
        self.refresh_route(id);
        let current = self.reload(id, false)?;
        Some((true, current))
    }

    fn replay_play(&self, id: &LoadId) {
        self.with_id(id, |state| {
            if state.is_playing(&id.key) {
                self.inner.runner.send(id.slot, SlotCall::Play);
            }
        });
    }
}
