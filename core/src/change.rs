use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

#[derive(Default)]
pub struct Change {
    wake: Notify,
    is_wanted: AtomicBool,
}

impl Change {
    pub fn new() -> Arc<Change> {
        Arc::new(Change::default())
    }

    pub fn mark(&self) {
        if self.is_wanted.load(Ordering::Acquire) {
            self.wake.notify_one();
        }
    }

    pub fn want(&self) {
        self.is_wanted.store(true, Ordering::Release);
        self.wake.notify_one();
    }

    pub fn release(&self) {
        self.is_wanted.store(false, Ordering::Release);
    }

    pub async fn changed(&self) {
        self.wake.notified().await;
    }
}
