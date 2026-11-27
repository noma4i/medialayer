use tokio::task::{JoinError, JoinHandle};
use std::future::Future;

pub fn restart<F, Fut>(handle: &tokio::runtime::Handle, name: &'static str, task: F) -> JoinHandle<()>
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let runtime = handle.clone();
    handle.spawn(async move {
        let mut delay = std::time::Duration::from_millis(100);
        loop {
            let started = tokio::time::Instant::now();
            let mut child = AbortOnDrop::new(runtime.spawn(task()));
            match child.join().await {
                Err(error) if error.is_panic() => {
                    crate::trace!("core", "heal task-restart name={name}");
                    if started.elapsed() >= std::time::Duration::from_secs(60) {
                        delay = std::time::Duration::from_millis(100);
                    }
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(std::time::Duration::from_secs(5));
                }
                _ => return,
            }
        }
    })
}

pub struct AbortOnDrop<T = ()>(JoinHandle<T>);

impl<T> AbortOnDrop<T> {
    pub fn new(task: JoinHandle<T>) -> AbortOnDrop<T> {
        AbortOnDrop(task)
    }

    pub fn abort(&self) {
        self.0.abort();
    }

    #[cfg(test)]
    pub fn abort_handle(&self) -> tokio::task::AbortHandle {
        self.0.abort_handle()
    }

    pub async fn join(&mut self) -> Result<T, JoinError> {
        (&mut self.0).await
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    use std::time::Duration;

    #[tokio::test(start_paused = true)]
    async fn inv_heal_16_restart_backoff_and_normal_return() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let count = Arc::new(AtomicUsize::new(0));
        let task = restart(&tokio::runtime::Handle::current(), "test", {
            let count = count.clone();
            move || {
                let n = count.fetch_add(1, Ordering::SeqCst);
                let tx = tx.clone();
                async move { tx.send(n).unwrap(); if n < 8 { panic!("test panic"); } }
            }
        });
        assert_eq!(rx.recv().await, Some(0));
        let start = tokio::time::Instant::now();
        let mut elapsed = Duration::ZERO;
        for (n, ms) in [100, 200, 400, 800, 1600, 3200, 5000, 5000].into_iter().enumerate() {
            tokio::task::yield_now().await;
            tokio::time::advance(Duration::from_millis(ms - 1)).await;
            assert_eq!(count.load(Ordering::SeqCst), n + 1);
            tokio::time::advance(Duration::from_millis(1)).await;
            assert_eq!(rx.recv().await, Some(n + 1));
            elapsed += Duration::from_millis(ms);
            assert_eq!(tokio::time::Instant::now() - start, elapsed);
        }
        task.await.unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 9);
    }

    #[tokio::test(start_paused = true)]
    async fn inv_heal_16_abort_stops_restart() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let task = restart(&tokio::runtime::Handle::current(), "test", move || {
            let tx = tx.clone();
            async move { tx.send(()).unwrap(); std::future::pending::<()>().await }
        });
        assert_eq!(rx.recv().await, Some(()));
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(rx.recv().await, None);
    }

    #[tokio::test(start_paused = true)]
    async fn inv_heal_16_backoff_resets_after_a_minute_alive() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let count = Arc::new(AtomicUsize::new(0));
        let task = restart(&tokio::runtime::Handle::current(), "test", {
            let count = count.clone();
            move || {
                let n = count.fetch_add(1, Ordering::SeqCst);
                let tx = tx.clone();
                async move {
                    tx.send(n).unwrap();
                    if n == 1 { tokio::time::sleep(Duration::from_secs(60)).await; }
                    if n < 2 { panic!("test panic"); }
                }
            }
        });
        assert_eq!(rx.recv().await, Some(0));
        assert_eq!(rx.recv().await, Some(1));
        let start = tokio::time::Instant::now();
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(99)).await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(rx.recv().await, Some(2));
        assert_eq!(tokio::time::Instant::now() - start, Duration::from_millis(60_100));
        task.await.unwrap();
    }
}
