use super::hub::{Deliver, Hub, Subscription};
use super::types::MediaState;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::runtime::Handle;

struct Place {
    state: MediaState,
    is_declared: bool,
    hub: Option<Hub<MediaState>>,
}

#[cfg(test)]
#[path = "places_cleanup_tests.rs"]
mod cleanup_tests;

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
    entries: Arc<Mutex<HashMap<String, Place>>>,
}

impl Places {
    pub fn new(handle: Handle) -> Places {
        Places { handle, entries: Arc::new(Mutex::new(HashMap::new())) }
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
        let (entries, key) = (Arc::downgrade(&self.entries), key.to_string());
        Subscription::cancelling(move || {
            drop(subscription);
            let Some(entries) = entries.upgrade() else { return };
            let mut entries = entries.lock().unwrap_or_else(PoisonError::into_inner);
            if entries.get(&key).is_some_and(Place::is_unused) {
                entries.remove(&key);
            }
        })
    }

    pub fn is_declared(&self, key: &str) -> bool {
        self.entries().get(key).is_some_and(|place| place.is_declared)
    }

    pub fn declared(&self, key: &str, is_cached: bool) {
        self.with(key, |place| {
            place.is_declared = true;
            match (&place.state, is_cached) {
                (state, _) if state.is_ready() => {}
                (MediaState::Loading, false) => {}
                (_, true) => place.move_to(MediaState::Ready),
                (_, false) => place.move_to(MediaState::Loading),
            }
        });
    }

    pub fn item_state(&self, key: &str, state: MediaState) {
        self.with(key, |place| match (&place.state, state) {
            (current, MediaState::Loading) if current.is_ready() => {}
            (_, state) => place.move_to(state),
        });
    }

    pub fn ready(&self, key: &str) {
        self.with(key, |place| {
            if !place.state.is_ready() {
                place.move_to(MediaState::Ready);
            }
        });
    }

    pub fn rendered(&self, key: &str) {
        self.with(key, |place| {
            if place.state != MediaState::Ended {
                place.move_to(MediaState::Rendered);
            }
        });
    }

    pub fn ended(&self, key: &str) {
        self.with(key, |place| {
            if place.state.is_ready() {
                place.move_to(MediaState::Ended);
            }
        });
    }

    pub fn resumed(&self, key: &str) {
        self.with(key, |place| {
            if place.state == MediaState::Ended {
                place.move_to(MediaState::Rendered);
            }
        });
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
        let place = entries.entry(key.to_string()).or_insert_with(Place::idle);
        let before = place.state.clone();
        change(place);
        if place.state != before {
            crate::trace!("place", "{} {before:?} -> {:?}", crate::trace::tail(key), place.state);
        }
        if entries.get(key).is_some_and(Place::is_unused) {
            entries.remove(key);
        }
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<String, Place>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
