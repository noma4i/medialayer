use crate::api::Kind;
use std::sync::{Arc, Mutex, PoisonError};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FailureStage {
    Load,
    Store,
    /// The player gave up on a video or audio after its retries.
    Play,
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
    state: Mutex<Option<Listeners>>,
    next: Mutex<u64>,
}

/// One listener with its own ordered mailbox: a slow listener delays only its own events (spec M6).
struct Listener {
    id: u64,
    target: Arc<dyn FailureListener>,
    active: Arc<AtomicBool>,
    mailbox: Arc<Mailbox>,
}

type Listeners = Vec<Listener>;

#[derive(Default)]
struct Mailbox {
    queue: Mutex<(VecDeque<Option<MediaFailure>>, bool)>,
}

impl Mailbox {
    /// Queues an event; the first push of a burst starts one drain thread that delivers in order and ends when the queue is empty.
    fn push(self: &Arc<Self>, listener: &Arc<dyn FailureListener>, active: &Arc<AtomicBool>, event: Option<MediaFailure>) {
        let mut queue = self.queue.lock().unwrap_or_else(PoisonError::into_inner);
        queue.0.push_back(event);
        if queue.1 { return; }
        queue.1 = true;
        drop(queue);
        let (mailbox, listener, active) = (self.clone(), listener.clone(), active.clone());
        std::thread::spawn(move || loop {
            let next = {
                let mut queue = mailbox.queue.lock().unwrap_or_else(PoisonError::into_inner);
                match queue.0.pop_front() {
                    Some(event) => event,
                    None => {
                        queue.1 = false;
                        return;
                    }
                }
            };
            if !active.load(Ordering::SeqCst) { continue; }
            match next {
                Some(failure) => listener.failed(failure),
                None => listener.closed(),
            }
        });
    }
}

impl Default for Failures {
    fn default() -> Self {
        Self { state: Mutex::new(Some(Vec::new())), next: Mutex::new(0) }
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
        if let Some(listeners) = state.as_mut() {
            listeners.push(Listener { id, target: listener, active: active.clone(), mailbox: Arc::default() });
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
                if let Some(listeners) = failures.state.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
                    listeners.retain(|held| held.id != id);
                }
            }
        })
    }

    pub fn report(&self, failure: MediaFailure) {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(listeners) = state.as_ref() else { return };
        for held in listeners {
            held.mailbox.push(&held.target, &held.active, Some(failure.clone()));
        }
    }

    pub fn close(&self) {
        let Some(listeners) = self.state.lock().unwrap_or_else(PoisonError::into_inner).take() else { return };
        for held in listeners {
            held.mailbox.push(&held.target, &held.active, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct Sleepy(Duration);
    impl FailureListener for Sleepy {
        fn failed(&self, _: MediaFailure) { std::thread::sleep(self.0); }
        fn closed(&self) {}
    }

    #[derive(Default)]
    struct Log(Mutex<Vec<String>>);
    impl FailureListener for Log {
        fn failed(&self, failure: MediaFailure) { self.0.lock().unwrap().push(failure.url); }
        fn closed(&self) { self.0.lock().unwrap().push("closed".into()); }
    }

    fn failure(url: &str) -> MediaFailure {
        MediaFailure { kind: Kind::Image, url: url.into(), stage: FailureStage::Load, reason: String::new() }
    }

    #[test]
    fn inv_api_67_a_slow_failure_listener_delays_neither_the_producer_nor_the_others_and_order_holds() {
        let failures = Arc::new(Failures::default());
        let _slow = failures.subscribe(Arc::new(Sleepy(Duration::from_millis(400))));
        let log = Arc::new(Log::default());
        let _fast = failures.subscribe(log.clone());
        let started = Instant::now();
        for index in 0..5 {
            failures.report(failure(&index.to_string()));
        }
        failures.close();
        assert!(started.elapsed() < Duration::from_millis(200));
        let deadline = Instant::now() + Duration::from_secs(5);
        while log.0.lock().unwrap().len() < 6 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*log.0.lock().unwrap(), vec!["0", "1", "2", "3", "4", "closed"]);
    }
}
