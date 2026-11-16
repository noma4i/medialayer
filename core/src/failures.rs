use crate::api::Kind;
use std::sync::{Arc, Mutex, PoisonError};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FailureStage {
    Load,
    Store,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MediaFailure {
    pub kind: Kind,
    pub url: String,
    pub stage: FailureStage,
    pub reason: String,
}

#[uniffi::export(with_foreign)]
pub trait FailureListener: Send + Sync {
    fn failed(&self, failure: MediaFailure);
    fn closed(&self);
}

pub struct Failures {
    state: Mutex<FailureState>,
    next: Mutex<u64>,
}

type Listeners = Vec<(u64, Arc<dyn FailureListener>, Arc<AtomicBool>)>;

#[derive(Default)]
struct FailureState {
    listeners: Option<Listeners>,
    pending: VecDeque<(Listeners, Option<MediaFailure>)>,
    delivering: bool,
}

impl Default for Failures {
    fn default() -> Self {
        Self { state: Mutex::new(FailureState { listeners: Some(Vec::new()), ..FailureState::default() }), next: Mutex::new(0) }
    }
}

impl std::fmt::Debug for Failures {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Failures")
    }
}

impl Failures {
    pub fn subscribe(self: &Arc<Failures>, listener: Arc<dyn FailureListener>) -> crate::api::Subscription {
        let id = {
            let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
            *next += 1;
            *next
        };
        let active = Arc::new(AtomicBool::new(true));
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(listeners) = state.listeners.as_mut() {
            listeners.push((id, listener, active.clone()));
        } else {
            drop(state);
            listener.closed();
            return crate::api::Subscription::cancelling(|| {});
        }
        drop(state);
        let failures = Arc::downgrade(self);
        crate::api::Subscription::cancelling(move || {
            active.store(false, Ordering::SeqCst);
            if let Some(failures) = failures.upgrade() {
                if let Some(listeners) = failures.state.lock().unwrap_or_else(PoisonError::into_inner).listeners.as_mut() {
                    listeners.retain(|(held, _, _)| *held != id);
                }
            }
        })
    }

    pub fn report(&self, failure: MediaFailure) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(listeners) = state.listeners.clone() else { return };
        state.pending.push_back((listeners, Some(failure)));
        Self::deliver(&self.state, state);
    }

    pub fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(listeners) = state.listeners.take() else { return };
        state.pending.push_back((listeners, None));
        Self::deliver(&self.state, state);
    }

    fn deliver<'a>(shared: &'a Mutex<FailureState>, mut state: std::sync::MutexGuard<'a, FailureState>) {
        if state.delivering { return; }
        state.delivering = true;
        while let Some((listeners, failure)) = state.pending.pop_front() {
            drop(state);
            for (_, listener, active) in listeners {
                if !active.load(Ordering::SeqCst) { continue; }
                if let Some(failure) = &failure { listener.failed(failure.clone()); } else { listener.closed(); }
            }
            state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        }
        state.delivering = false;
    }
}
