use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

pub(super) struct Gate {
    limit: AtomicUsize,
    running: AtomicUsize,
    wake: Notify,
}

pub(super) struct LoadPermit(Arc<Gate>);

impl Drop for LoadPermit {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, Ordering::SeqCst);
        self.0.wake.notify_waiters();
    }
}

impl Gate {
    pub fn new(limit: usize) -> Arc<Gate> {
        Arc::new(Gate { limit: AtomicUsize::new(limit), running: AtomicUsize::new(0), wake: Notify::new() })
    }

    pub fn set_limit(&self, limit: usize) {
        self.limit.store(limit, Ordering::SeqCst);
        self.wake.notify_waiters();
    }

    pub async fn acquire(self: &Arc<Gate>) -> LoadPermit {
        loop {
            let notified = self.wake.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.try_take() {
                return LoadPermit(self.clone());
            }
            notified.await;
        }
    }

    fn try_take(&self) -> bool {
        self.running
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |running| (running < self.limit.load(Ordering::SeqCst)).then_some(running + 1))
            .is_ok()
    }
}
