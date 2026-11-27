use tokio::task::{JoinError, JoinHandle};
use std::future::Future;

pub fn restart<F, Fut>(handle: &tokio::runtime::Handle, name: &'static str, task: F) -> JoinHandle<()>
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let runtime = handle.clone();
    handle.spawn(async move {
        loop {
            let mut child = AbortOnDrop::new(runtime.spawn(task()));
            match child.join().await {
                Err(error) if error.is_panic() => crate::trace!("core", "heal task-restart name={name}"),
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
