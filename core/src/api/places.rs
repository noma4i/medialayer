use super::hub::{Deliver, Hub, Subscription};
use super::types::MediaState;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use tokio::runtime::Handle;

struct Place {
    state: MediaState,
    is_declared: bool,
    hub: Option<Hub<MediaState>>,
}

impl Place {
    fn idle() -> Place {
        Place { state: MediaState::Idle, is_declared: false, hub: None }
    }

    fn move_to(&mut self, state: MediaState) {
        if self.state == state {
            return;
        }
        self.state = state.clone();
        if let Some(hub) = &self.hub {
            hub.publish(state);
        }
    }

    fn is_unused(&self) -> bool {
        !self.is_declared && self.state == MediaState::Idle && self.hub.as_ref().is_none_or(|hub| hub.listener_count() == 0)
    }
}

pub struct Places {
    handle: Handle,
    entries: Mutex<HashMap<String, Place>>,
}

impl Places {
    pub fn new(handle: Handle) -> Places {
        Places { handle, entries: Mutex::new(HashMap::new()) }
    }

    pub fn state(&self, key: &str) -> MediaState {
        self.entries().get(key).map_or(MediaState::Idle, |place| place.state.clone())
    }

    pub fn subscribe(&self, key: &str, deliver: Deliver<MediaState>) -> Subscription {
        let mut entries = self.entries();
        let place = entries.entry(key.to_string()).or_insert_with(Place::idle);
        let hub = place.hub.get_or_insert_with(|| Hub::new(self.handle.clone()));
        let subscription = hub.subscribe(deliver);
        hub.publish(place.state.clone());
        subscription
    }

    pub fn declared(&self, key: &str, is_cached: bool) {
        self.with(key, |place| {
            place.is_declared = true;
            match (&place.state, is_cached) {
                (MediaState::Ready, _) | (MediaState::Loading, false) => {}
                (_, true) => place.move_to(MediaState::Ready),
                (_, false) => place.move_to(MediaState::Loading),
            }
        });
    }

    pub fn ready(&self, key: &str) {
        self.with(key, |place| place.move_to(MediaState::Ready));
    }

    pub fn failed(&self, key: &str, detail: &str) {
        self.with(key, |place| place.move_to(MediaState::Failed { detail: detail.to_string() }));
    }

    pub fn unloaded(&self, key: &str) {
        self.with(key, |place| {
            let next = if place.is_declared { MediaState::Loading } else { MediaState::Idle };
            place.move_to(next);
        });
    }

    pub fn retired(&self, key: &str) {
        self.with(key, |place| {
            place.is_declared = false;
            place.move_to(MediaState::Idle);
        });
    }

    pub fn retire_all(&self) {
        let keys: Vec<String> = self.entries().keys().cloned().collect();
        for key in keys {
            self.retired(&key);
        }
    }

    fn with(&self, key: &str, change: impl FnOnce(&mut Place)) {
        let mut entries = self.entries();
        change(entries.entry(key.to_string()).or_insert_with(Place::idle));
        if entries.get(key).is_some_and(Place::is_unused) {
            entries.remove(key);
        }
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<String, Place>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
