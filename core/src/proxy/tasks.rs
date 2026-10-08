use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::task::{AbortHandle, JoinSet};

#[derive(Clone, Default)]
pub(super) struct Tasks {
    set: Arc<Mutex<JoinSet<()>>>,
}

impl Tasks {
    pub fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) -> AbortHandle {
        let mut set = self.set.lock().unwrap_or_else(PoisonError::into_inner);
        while set.try_join_next().is_some() {}
        set.spawn(future)
    }
}

pub(super) struct TasksOwner(pub Tasks);

impl Drop for TasksOwner {
    fn drop(&mut self) {
        self.0.set.lock().unwrap_or_else(PoisonError::into_inner).abort_all();
    }
}
