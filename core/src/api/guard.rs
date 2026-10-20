use tokio::task::{JoinError, JoinHandle};

pub struct AbortOnDrop<T = ()>(JoinHandle<T>);

impl<T> AbortOnDrop<T> {
    pub fn new(task: JoinHandle<T>) -> AbortOnDrop<T> {
        AbortOnDrop(task)
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
