use crate::api::{MediaError, Subscription};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::watch;

type Declaration<T> = Box<dyn FnOnce(&T) + Send>;

struct Queue<T> {
    value: Option<Arc<T>>,
    draining: bool,
    failed: bool,
    waiting: Vec<Declaration<T>>,
}

pub struct Deferred<T> {
    queue: Mutex<Queue<T>>,
    outcome: watch::Sender<Option<Result<Arc<T>, MediaError>>>,
}

#[derive(Default)]
struct Held {
    cancelled: bool,
    subscription: Option<Subscription>,
}

fn locked<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl<T: Send + Sync + 'static> Deferred<T> {
    pub fn new() -> Arc<Deferred<T>> {
        let (outcome, _) = watch::channel(None);
        Arc::new(Deferred { queue: Mutex::new(Queue { value: None, draining: false, failed: false, waiting: Vec::new() }), outcome })
    }

    pub fn run(&self, work: impl FnOnce(&T) + Send + 'static) {
        let mut queue = locked(&self.queue);
        match &queue.value {
            Some(value) if !queue.draining => {
                let value = value.clone();
                drop(queue);
                work(&value);
            }
            _ if queue.failed => {}
            _ => queue.waiting.push(Box::new(work)),
        }
    }

    pub fn install(&self, value: Arc<T>) {
        let mut queue = locked(&self.queue);
        queue.value = Some(value.clone());
        queue.draining = true;
        loop {
            let waiting = std::mem::take(&mut queue.waiting);
            if waiting.is_empty() {
                queue.draining = false;
                break;
            }
            drop(queue);
            waiting.into_iter().for_each(|work| work(&value));
            queue = locked(&self.queue);
        }
        drop(queue);
        self.outcome.send_replace(Some(Ok(value)));
    }

    pub fn fail(&self, error: MediaError) {
        let mut queue = locked(&self.queue);
        queue.failed = true;
        queue.waiting.clear();
        drop(queue);
        self.outcome.send_replace(Some(Err(error)));
    }

    pub async fn wait(&self) -> Result<Arc<T>, MediaError> {
        let mut outcome = self.outcome.subscribe();
        let settled = outcome.wait_for(Option::is_some).await.map_err(|error| MediaError::Unexpected { detail: error.to_string() })?;
        settled.clone().expect("a settled outcome holds a value")
    }

    pub fn hold(&self, subscribe: impl FnOnce(&T) -> Subscription + Send + 'static) -> Subscription {
        let held = Arc::new(Mutex::new(Held::default()));
        let fill = held.clone();
        self.run(move |value| {
            let mut held = locked(&fill);
            if !held.cancelled {
                held.subscription = Some(subscribe(value));
            }
        });
        Subscription::cancelling(move || {
            let mut held = locked(&held);
            held.cancelled = true;
            held.subscription.take();
        })
    }
}
