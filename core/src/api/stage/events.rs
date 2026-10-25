use super::Stage;
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
        for (key, action) in actions {
            self.perform(key, action);
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
        for (key, action) in actions {
            self.perform(key, action);
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

    pub(super) fn feed(&self, key: &str, event: Event) {
        let mut actions = Vec::new();
        self.advance(&mut self.state(), key, event, &mut actions);
        for (key, action) in actions {
            self.perform(key, action);
        }
    }

    pub(super) fn perform(&self, key: String, action: Action) {
        crate::trace!("event", "action {} {action:?}", crate::trace::tail(&key));
        match action {
            Action::None => {}
            Action::ForegroundRetry(delay_ms) => self.wake_after(key, delay_ms, Wake::Foreground),
            Action::Arm { delay_ms } => self.wake_after(key, delay_ms, Wake::Watchdog),
            Action::Retry(delay_ms) => self.wake_after(key, delay_ms, Wake::Retry),
            Action::Disarm => self.cancel_task(&key),
            Action::Replay => self.reload(&key),
            Action::Recover => {
                let stage = self.clone();
                self.inner.handle.spawn(async move {
                    stage.inner.routes.repair().await;
                    stage.refresh_stale_route(&key);
                    stage.reload(&key);
                });
            }
            Action::PurgeCache(url) => {
                let stage = self.clone();
                self.inner.handle.spawn(async move {
                    stage.inner.routes.purge(&url).await;
                    stage.feed(&key, Event::CachePurged);
                });
            }
            Action::RefreshRoute => {
                if let Some(loaded) = self.state().loaded.get_mut(&key) {
                    loaded.route = self.inner.routes.play_url(&loaded.source);
                }
                self.feed(&key, Event::RouteRefreshed);
            }
            Action::Fail(reason) => self.inner.places.failed(&key, &reason),
            Action::ForegroundPass => {
                let is_ok = self.foreground_pass(&key);
                self.feed(&key, Event::ForegroundPassDone { is_ok });
            }
        }
    }

    fn cancel_task(&self, key: &str) {
        self.state().tasks.remove(key);
    }

    fn wake_after(&self, key: String, delay_ms: u64, wake: Wake) {
        let stage = self.clone();
        let target = key.clone();
        let task = self.inner.handle.spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            if wake != Wake::Watchdog {
                stage.inner.routes.repair().await;
            }
            match wake {
                Wake::Watchdog => stage.sample_watchdog(&target).await,
                Wake::Retry => stage.retry_due(&target),
                Wake::Foreground => stage.feed(&target, Event::ForegroundDue),
            }
        });
        self.state().tasks.insert(key, AbortOnDrop::new(task));
    }

    async fn sample_watchdog(&self, key: &str) {
        let Some(slot) = self.state().loaded.get(key).map(|loaded| loaded.slot) else { return };
        let Some(SlotSample { position_ms, duration_ms, is_looping, view_attached, frame_rendered }) = self.inner.runner.sample(slot).await
        else {
            return;
        };
        let sample = Sample { position_ms, duration_ms, is_looping, view_attached, frame_rendered };
        self.feed(key, Event::WatchdogFired(sample));
    }

    fn retry_due(&self, key: &str) {
        let Some(resolved_generation) = self.state().loaded.get(key).map(|loaded| loaded.route.generation) else { return };
        let event = Event::RetryDue {
            resolved_generation,
            current_generation: self.inner.routes.generation(),
            readiness: self.inner.routes.readiness(),
        };
        self.feed(key, event);
    }

    fn refresh_stale_route(&self, key: &str) {
        let mut state = self.state();
        let Some(loaded) = state.loaded.get_mut(key) else { return };
        loaded.route = self.inner.routes.refresh(&loaded.source, &loaded.route);
    }

    pub(super) fn reload(&self, key: &str) {
        let (state, calls) = {
            let mut state = self.state();
            let Some(loaded) = state.loaded.get(key) else { return };
            let slot = loaded.slot;
            let generation = state.next_generation();
            if let Some(loaded) = state.loaded.get_mut(key) {
                loaded.generation = generation;
            }
            let mut calls = vec![(slot, SlotCall::Unload)];
            let resume = state.positions.take(key);
            calls.extend(self.load_calls(&state, key, resume));
            if state.is_playing(key) {
                calls.push((slot, SlotCall::Play));
            }
            (state, calls)
        };
        let is_playing = calls.iter().any(|(_, call)| *call == SlotCall::Play);
        for (slot, call) in calls {
            self.inner.runner.send(slot, call);
        }
        drop(state);
        self.refresh_hints();
        if is_playing {
            self.feed(key, Event::Play { position_ms: 0 });
        }
    }

    fn foreground_pass(&self, key: &str) -> bool {
        self.inner.foreground_passes.fetch_add(1, Ordering::SeqCst);
        if self.inner.routes.readiness() != Readiness::Ready {
            return false;
        }
        let stale = self.state().loaded.get(key).is_some_and(|loaded| self.inner.routes.is_stale(&loaded.route));
        if stale {
            self.perform(key.to_string(), Action::RefreshRoute);
            self.reload(key);
        } else {
            self.replay_play(key);
        }
        true
    }

    fn replay_play(&self, key: &str) {
        let state = self.state();
        let slot = state.loaded.get(key).filter(|_| state.is_playing(key)).map(|loaded| loaded.slot);
        if let Some(slot) = slot {
            self.inner.runner.send(slot, SlotCall::Play);
        }
    }
}
