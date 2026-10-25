use super::{Armed, LoadId, Stage, StateGuard};
use crate::api::guard::AbortOnDrop;
use crate::api::slots::{SlotCall, SlotSample};
use crate::api::types::MediaState;
use crate::proxy::Readiness;
use crate::session::{Action, Event, Sample};
use std::sync::atomic::Ordering;
use std::time::Duration;

#[derive(PartialEq)]
enum Wake {
    Watchdog,
    Retry,
    Foreground,
}

impl Stage {
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

    fn current_key(state: &super::State, slot: usize, generation: u64) -> Option<String> {
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
            let Some(key) = Self::current_key(&state, slot, generation) else { return };
            self.advance(&mut state, &key, Event::Pause, &mut actions);
            self.inner.places.ended(&key);
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

    pub(super) fn with_id<R>(&self, id: &LoadId, work: impl FnOnce(&mut StateGuard<'_>) -> R) -> Option<R> {
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
                self.with_id(&id, |state| state.tasks.remove(&id.key));
            }
            Action::Replay => {
                self.reload(&id, false);
            }
            Action::Recover => {
                let stage = self.clone();
                self.inner.handle.spawn(async move {
                    stage.inner.routes.repair().await;
                    stage.reload(&id, true);
                });
            }
            Action::PurgeCache(url) => {
                let stage = self.clone();
                self.inner.handle.spawn(async move {
                    stage.inner.routes.purge(&url).await;
                    stage.feed_id(&id, Event::CachePurged);
                });
            }
            Action::RefreshRoute => self.refresh_route(&id),
            Action::Fail(reason) => {
                self.with_id(&id, |_| self.inner.places.failed(&id.key, &reason));
            }
            Action::ForegroundPass => {
                if let Some((is_ok, current)) = self.foreground_pass(&id) {
                    self.feed_id(&current, Event::ForegroundPassDone { is_ok });
                }
            }
        }
    }

    fn wake_after(&self, id: LoadId, delay_ms: u64, wake: Wake) {
        let stage = self.clone();
        let target = id.clone();
        let task = self.inner.handle.spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            if wake != Wake::Watchdog {
                stage.inner.routes.repair().await;
            }
            match wake {
                Wake::Watchdog => stage.sample_watchdog(&target).await,
                Wake::Retry => stage.retry_due(&target),
                Wake::Foreground => {
                    stage.feed_id(&target, Event::ForegroundDue);
                }
            }
        });
        let armed = Armed { generation: id.generation, _task: AbortOnDrop::new(task) };
        self.with_id(&id, |state| state.tasks.insert(id.key.clone(), armed));
    }

    async fn sample_watchdog(&self, id: &LoadId) {
        if self.with_id(id, |_| ()).is_none() {
            return;
        }
        let Some(SlotSample { position_ms, duration_ms, is_looping, view_attached, frame_rendered }) = self.inner.runner.sample(id.slot).await
        else {
            return;
        };
        let sample = Sample { position_ms, duration_ms, is_looping, view_attached, frame_rendered };
        self.feed_id(id, Event::WatchdogFired(sample));
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
            if let Some(loaded) = state.loaded.get_mut(key) {
                loaded.generation = generation;
            }
            let mut calls = vec![(id.slot, SlotCall::Unload)];
            let resume = state.positions.take(key);
            calls.extend(self.load_calls(&state, key, resume));
            if state.is_playing(key) {
                calls.push((id.slot, SlotCall::Play));
            }
            (state, calls, LoadId { key: id.key.clone(), slot: id.slot, generation })
        };
        let is_playing = calls.iter().any(|(_, call)| *call == SlotCall::Play);
        for (slot, call) in calls {
            self.inner.runner.send(slot, call);
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
