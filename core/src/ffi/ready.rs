use crate::api::MediaError;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::watch;

struct State<T> {
    value: Option<Arc<T>>,
    ended: bool,
}

pub struct Ready<T> {
    state: Mutex<State<T>>,
    outcome: watch::Sender<Option<Result<Arc<T>, MediaError>>>,
}

fn locked<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl<T: Send + Sync + 'static> Ready<T> {
    pub fn new() -> Arc<Ready<T>> {
        let (outcome, _) = watch::channel(None);
        Arc::new(Ready { state: Mutex::new(State { value: None, ended: false }), outcome })
    }

    /// Returns the value back when the owner already ended.
    pub fn install(&self, value: Arc<T>) -> Option<Arc<T>> {
        let mut state = locked(&self.state);
        if state.ended {
            return Some(value);
        }
        state.value = Some(value.clone());
        self.outcome.send_replace(Some(Ok(value)));
        None
    }

    pub fn fail(&self, error: MediaError) {
        drop(self.end(error));
    }

    pub fn close(&self) -> Option<Arc<T>> {
        self.end(MediaError::Closed)
    }

    fn end(&self, error: MediaError) -> Option<Arc<T>> {
        let mut state = locked(&self.state);
        if state.ended {
            return None;
        }
        state.ended = true;
        self.outcome.send_replace(Some(Err(error)));
        state.value.take()
    }

    pub async fn wait(&self) -> Result<Arc<T>, MediaError> {
        let mut outcome = self.outcome.subscribe();
        let settled = outcome.wait_for(Option::is_some).await.map_err(|error| MediaError::Unexpected { detail: error.to_string() })?;
        settled.clone().expect("a settled outcome holds a value")
    }
}
