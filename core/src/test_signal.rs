use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::Notify;

pub async fn timeout<T>(limit: std::time::Duration, future: impl std::future::Future<Output = T>) -> Result<T, ()> {
    let (cancel, cancelled) = std::sync::mpsc::channel::<()>();
    let (expired, deadline) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        if matches!(cancelled.recv_timeout(limit), Err(std::sync::mpsc::RecvTimeoutError::Timeout)) {
            let _ = expired.send(());
        }
    });
    let result = tokio::select! {
        value = future => Ok(value),
        _ = deadline => Err(()),
    };
    drop(cancel);
    result
}

static VERSION: AtomicU64 = AtomicU64::new(0);
static CHANGED: Notify = Notify::const_new();

pub fn fire() {
    VERSION.fetch_add(1, Ordering::SeqCst);
    CHANGED.notify_waiters();
}

pub fn current() -> u64 {
    VERSION.load(Ordering::SeqCst)
}

pub async fn changed_since(seen: u64) {
    let notified = CHANGED.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    if current() == seen {
        notified.await;
    }
}
