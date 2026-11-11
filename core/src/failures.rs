use crate::api::Kind;
use std::sync::{Arc, Mutex, PoisonError};

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
}

#[derive(Default)]
pub struct Failures {
    listeners: Mutex<Vec<(u64, Arc<dyn FailureListener>)>>,
    next: Mutex<u64>,
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
        self.listeners.lock().unwrap_or_else(PoisonError::into_inner).push((id, listener));
        let failures = Arc::downgrade(self);
        crate::api::Subscription::cancelling(move || {
            if let Some(failures) = failures.upgrade() {
                failures.listeners.lock().unwrap_or_else(PoisonError::into_inner).retain(|(held, _)| *held != id);
            }
        })
    }

    pub fn report(&self, failure: MediaFailure) {
        let listeners: Vec<_> = self.listeners.lock().unwrap_or_else(PoisonError::into_inner).iter().map(|(_, listener)| listener.clone()).collect();
        for listener in listeners {
            listener.failed(failure.clone());
        }
    }
}
