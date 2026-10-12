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
    pub fn ready(&self, slot: usize) {
        crate::trace!("event", "ready slot={slot} key={:?}", self.key_at(slot).as_deref().map(crate::trace::tail));
        let Some(key) = self.key_at(slot) else { return };
        self.feed(&key, Event::Loaded);
        self.inner.places.ready(&key);
        self.sync_foreground();
    }

    pub fn visual_ready(&self, slot: usize) {
        crate::trace!("event", "visual_ready slot={slot} key={:?}", self.key_at(slot).as_deref().map(crate::trace::tail));
        if let Some(key) = self.key_at(slot) {
            self.inner.places.rendered(&key);
        }
        self.sync_foreground();
    }

    pub fn stalled(&self, slot: usize, position_ms: u64) {
        crate::trace!("event", "stalled slot={slot} position={position_ms} key={:?}", self.key_at(slot).as_deref().map(crate::trace::tail));
        let Some(key) = self.key_at(slot).filter(|key| self.state().is_playing(key)) else { return };
        self.feed(&key, Event::Play { position_ms });
    }

    pub fn failed(&self, slot: usize, status: i32, detail: String) {
        crate::trace!("event", "failed slot={slot} status={status} detail={detail} key={:?}", self.key_at(slot).as_deref().map(crate::trace::tail));
        let Some(key) = self.key_at(slot) else { return };
        let event = if status > 0 { Event::HttpError { status } } else { Event::PlaybackFailed { reason: detail } };
        self.feed(&key, event);
        self.sync_foreground();
    }

    pub fn ended(&self, slot: usize) {
        if let Some(key) = self.key_at(slot) {
            self.feed(&key, Event::Pause);
            self.inner.places.ended(&key);
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

    fn key_at(&self, slot: usize) -> Option<String> {
        self.state().slot_keys.get(&slot).cloned()
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

    fn reload(&self, key: &str) {
        let calls = {
            let state = self.state();
            let Some(loaded) = state.loaded.get(key) else { return };
            let mut calls = vec![
                (loaded.slot, SlotCall::Unload),
                (loaded.slot, SlotCall::Load { key: key.to_string(), route: loaded.route.clone() }),
            ];
            if let Some(rate) = state.rates.get(key) {
                calls.push((loaded.slot, SlotCall::Rate { rate: *rate }));
            }
            if state.is_playing(key) {
                calls.push((loaded.slot, SlotCall::Play));
            }
            calls
        };
        let is_playing = calls.iter().any(|(_, call)| *call == SlotCall::Play);
        for (slot, call) in calls {
            self.inner.runner.send(slot, call);
        }
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
        let slot = {
            let state = self.state();
            state.loaded.get(key).filter(|_| state.is_playing(key)).map(|loaded| loaded.slot)
        };
        if let Some(slot) = slot {
            self.inner.runner.send(slot, SlotCall::Play);
        }
    }
}
