use super::Stage;
use crate::api::guard::AbortOnDrop;
use crate::api::slots::{SlotCall, SlotSample};
use crate::session::{Action, Event, Sample};
use std::time::Duration;

enum Wake {
    Watchdog,
    Retry,
}

impl Stage {
    pub fn ready(&self, slot: usize) {
        let Some(key) = self.key_at(slot) else { return };
        self.feed(&key, Event::Loaded);
        self.inner.places.ready(&key);
    }

    pub fn visual_ready(&self, slot: usize) {
        if let Some(key) = self.key_at(slot) {
            self.inner.places.ready(&key);
        }
    }

    pub fn stalled(&self, slot: usize, position_ms: u64) {
        let Some(key) = self.key_at(slot).filter(|key| self.state().is_playing(key)) else { return };
        self.feed(&key, Event::Play { position_ms });
    }

    pub fn failed(&self, slot: usize, status: i32, detail: String) {
        let Some(key) = self.key_at(slot) else { return };
        let event = if status > 0 { Event::HttpError { status } } else { Event::PlaybackFailed { reason: detail } };
        self.feed(&key, event);
    }

    pub fn ended(&self, slot: usize) {
        if let Some(key) = self.key_at(slot) {
            self.feed(&key, Event::Pause);
        }
    }

    pub(super) fn feed(&self, key: &str, event: Event) {
        let mut actions = Vec::new();
        self.advance(&mut self.state(), key, event, &mut actions);
        for (key, action) in actions {
            self.perform(key, action);
        }
    }

    pub(super) fn perform(&self, key: String, action: Action) {
        match action {
            Action::None | Action::ForegroundRetry(_) => {}
            Action::Arm { delay_ms } => self.wake_after(key, delay_ms, Wake::Watchdog),
            Action::Retry(delay_ms) => self.wake_after(key, delay_ms, Wake::Retry),
            Action::Disarm => self.cancel_task(&key),
            Action::Recover | Action::Replay => self.reload(&key),
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
                self.replay_play(&key);
                self.feed(&key, Event::ForegroundPassDone { is_ok: true });
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
            match wake {
                Wake::Watchdog => stage.sample_watchdog(&target).await,
                Wake::Retry => stage.retry_due(&target),
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
        let sample = Sample { position_ms, duration_ms, is_looping, loop_wrapped: false, view_attached, frame_rendered };
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

    fn reload(&self, key: &str) {
        let calls = {
            let state = self.state();
            let Some(loaded) = state.loaded.get(key) else { return };
            let mut calls = vec![
                (loaded.slot, SlotCall::Unload),
                (loaded.slot, SlotCall::Load { key: key.to_string(), route: loaded.route.clone() }),
            ];
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
