use super::hub::{Deliver, Hub, Subscription};
use super::places_state::{Lifecycle, Signal};
use super::types::MediaState;
use statig::blocking::{IntoStateMachineExt, StateMachine};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::runtime::Handle;

struct Place {
    machine: StateMachine<Lifecycle>,
    is_declared: bool,
    hub: Option<Hub<MediaState>>,
}

#[cfg(test)]
#[path = "places_cleanup_tests.rs"]
mod cleanup_tests;

impl Place {
    fn idle() -> Place {
        Place { machine: Lifecycle.state_machine(), is_declared: false, hub: None }
    }

    fn state(&self) -> MediaState {
        self.machine.state().into()
    }

    fn signal(&mut self, signal: Signal) -> bool {
        let before = self.state();
        self.machine.handle(&signal);
        let after = self.state();
        if after == before {
            return false;
        }
        if let Some(hub) = &self.hub {
            hub.publish(after);
        }
        true
    }

    fn is_unused(&self) -> bool {
        !self.is_owned() && self.state() == MediaState::Idle
    }

    fn is_owned(&self) -> bool {
        self.is_declared || self.hub.as_ref().is_some_and(|hub| hub.listener_count() > 0)
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
        self.entries().get(key).map_or(MediaState::Idle, Place::state)
    }

    pub fn subscribe(&self, key: &str, deliver: Deliver<MediaState>) -> Subscription {
        let mut entries = self.entries();
        let place = entries.entry(key.to_string()).or_insert_with(Place::idle);
        let state = place.state();
        let hub = place.hub.get_or_insert_with(|| Hub::new(self.handle.clone()));
        let subscription = hub.subscribe(deliver);
        hub.publish(state);
        let (entries, key) = (Arc::downgrade(&self.entries), key.to_string());
        Subscription::cancelling(move || {
            drop(subscription);
            let Some(entries) = entries.upgrade() else { return };
            let mut entries = entries.lock().unwrap_or_else(PoisonError::into_inner);
            if entries.get(&key).is_some_and(|place| !place.is_owned()) {
                entries.remove(&key);
            }
        })
    }

    pub fn is_declared(&self, key: &str) -> bool {
        self.entries().get(key).is_some_and(|place| place.is_declared)
    }

    pub fn claimed(&self, key: &str) {
        self.with(key, |place| place.is_declared = true);
    }

    pub fn declared(&self, key: &str, is_cached: bool) {
        self.with(key, |place| {
            place.is_declared = true;
            place.signal(if is_cached { Signal::Cached } else { Signal::Loading });
        });
    }

    pub fn item_state(&self, key: &str, state: MediaState) {
        let mut entries = self.entries();
        let Some(place) = entries.get_mut(key).filter(|place| place.is_owned()) else { return };
        let before = place.state();
        place.signal(Signal::from_item(state));
        Self::settle(&mut entries, key, &before);
    }

    pub fn ready(&self, key: &str) {
        self.with(key, |place| place.signal(Signal::Ready));
    }

    pub fn rendered(&self, key: &str) {
        self.with(key, |place| place.signal(Signal::Rendered));
    }

    pub fn ended(&self, key: &str) -> bool {
        self.with(key, |place| place.signal(Signal::Ended))
    }

    pub fn resumed(&self, key: &str) {
        self.with(key, |place| place.signal(Signal::Resumed));
    }

    pub fn failed(&self, key: &str, detail: &str) {
        self.with(key, |place| place.signal(Signal::Failed(detail.to_string())));
    }

    pub fn unloaded(&self, key: &str) {
        self.with(key, |place| {
            let is_declared = place.is_declared;
            place.signal(Signal::Unloaded { is_declared })
        });
    }

    pub fn retired(&self, key: &str) {
        self.with(key, |place| {
            place.is_declared = false;
            place.signal(Signal::Cleared)
        });
    }

    pub fn retire_all(&self) {
        let keys: Vec<String> = self.entries().keys().cloned().collect();
        for key in keys {
            self.retired(&key);
        }
    }

    fn with<R>(&self, key: &str, change: impl FnOnce(&mut Place) -> R) -> R {
        let mut entries = self.entries();
        let place = entries.entry(key.to_string()).or_insert_with(Place::idle);
        let before = place.state();
        let result = change(place);
        Self::settle(&mut entries, key, &before);
        result
    }

    fn settle(entries: &mut HashMap<String, Place>, key: &str, before: &MediaState) {
        let Some(place) = entries.get(key) else { return };
        let after = place.state();
        if after != *before {
            crate::trace!("place", "{} {before:?} -> {after:?}", crate::trace::tail(key));
        }
        if place.is_unused() {
            entries.remove(key);
        }
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<String, Place>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
