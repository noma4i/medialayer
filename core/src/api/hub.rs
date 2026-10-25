use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::runtime::Handle;
use tokio::sync::Notify;
use super::guard::AbortOnDrop;

pub type Deliver<T> = Arc<dyn Fn(T) + Send + Sync>;

struct Mailbox<T> {
    slot: Mutex<Option<T>>,
    wake: Notify,
    closed: AtomicBool,
}

struct Shared<T> {
    latest: Mutex<Option<T>>,
    wake: Notify,
    boxes: Mutex<HashMap<u64, Arc<Mailbox<T>>>>,
    next_id: AtomicU64,
    published: AtomicU64,
    #[cfg(test)]
    unsubscribe_visits: AtomicU64,
}

fn locked<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub struct Hub<T> {
    shared: Arc<Shared<T>>,
    handle: Handle,
    _dispatcher: AbortOnDrop,
}

#[derive(uniffi::Object)]
pub struct Subscription {
    cancel: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Subscription {
    pub fn cancelling(cancel: impl FnOnce() + Send + 'static) -> Subscription {
        Subscription { cancel: Mutex::new(Some(Box::new(cancel))) }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(cancel) = locked(&self.cancel).take() {
            cancel();
        }
    }
}

impl<T: Clone + Send + 'static> Hub<T> {
    pub fn new(handle: Handle) -> Hub<T> {
        let shared = Arc::new(Shared { latest: Mutex::new(None), wake: Notify::new(), boxes: Mutex::new(HashMap::new()), next_id: AtomicU64::new(0), published: AtomicU64::new(0), #[cfg(test)] unsubscribe_visits: AtomicU64::new(0) });
        let dispatcher = AbortOnDrop::new(handle.spawn(dispatch(shared.clone())));
        Hub { shared, handle, _dispatcher: dispatcher }
    }

    pub fn publish(&self, value: T) {
        self.shared.published.fetch_add(1, Ordering::Relaxed);
        *locked(&self.shared.latest) = Some(value);
        self.shared.wake.notify_one();
    }

    pub fn subscribe(&self, deliver: Deliver<T>) -> Subscription {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let mailbox = Arc::new(Mailbox { slot: Mutex::new(None), wake: Notify::new(), closed: AtomicBool::new(false) });
        locked(&self.shared.boxes).insert(id, mailbox.clone());
        let task = self.handle.spawn(deliver_latest(mailbox, deliver));
        let shared = Arc::downgrade(&self.shared);
        Subscription {
            cancel: Mutex::new(Some(Box::new(move || {
                task.abort();
                if let Some(shared) = shared.upgrade() {
                    locked(&shared.boxes).remove(&id);
                    #[cfg(test)]
                    shared.unsubscribe_visits.fetch_add(1, Ordering::Relaxed);
                }
            }))),
        }
    }

    #[cfg(test)]
    pub fn published(&self) -> u64 {
        self.shared.published.load(Ordering::Relaxed)
    }

    pub fn listener_count(&self) -> usize {
        locked(&self.shared.boxes).len()
    }

    #[cfg(test)]
    pub fn unsubscribe_visits(&self) -> u64 {
        self.shared.unsubscribe_visits.load(Ordering::Relaxed)
    }
}

impl<T> Drop for Hub<T> {
    fn drop(&mut self) {
        for mailbox in locked(&self.shared.boxes).values() {
            mailbox.closed.store(true, Ordering::SeqCst);
            mailbox.wake.notify_one();
        }
    }
}

async fn dispatch<T: Clone + Send + 'static>(shared: Arc<Shared<T>>) {
    loop {
        shared.wake.notified().await;
        let Some(value) = locked(&shared.latest).take() else {
            continue;
        };
        distribute(&shared, value);
    }
}

fn distribute<T: Clone>(shared: &Shared<T>, value: T) {
    for mailbox in locked(&shared.boxes).values() {
        *locked(&mailbox.slot) = Some(value.clone());
        mailbox.wake.notify_one();
    }
}

async fn deliver_latest<T: Send + 'static>(mailbox: Arc<Mailbox<T>>, deliver: Deliver<T>) {
    loop {
        mailbox.wake.notified().await;
        if mailbox.closed.load(Ordering::SeqCst) {
            return;
        }
        let Some(value) = locked(&mailbox.slot).take() else {
            continue;
        };
        let deliver = deliver.clone();
        let _ = tokio::task::spawn_blocking(move || deliver(value)).await;
    }
}

#[cfg(test)]
mod allocation_tests {
    use super::*;

    #[tokio::test]
    async fn inv_api_02_distribution_allocates_no_listener_snapshot() {
        let hub = Hub::new(Handle::current());
        let _subscriptions: Vec<_> = (0..64).map(|_| hub.subscribe(Arc::new(|_: u32| {}))).collect();
        distribute(&hub.shared, 6);
        assert_eq!(crate::alloc_probe::allocations_of(|| distribute(&hub.shared, 7)), 0);
    }
}
