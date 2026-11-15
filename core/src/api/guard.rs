use tokio::task::{JoinError, JoinHandle};

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
