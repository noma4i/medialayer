use crate::api::{MediaError, Subscription};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::watch;

type Declaration<T> = Box<dyn FnOnce(&T) + Send>;

const WAITING_EVENT_CAPACITY: usize = 256;

enum Rule {
    Event,
    Subscription,
    Latest,
    Opens,
    Closes,
}

enum Plan {
    Push(String, Rule),
    Drop,
}

struct Waiting<T> {
    id: u64,
    name: String,
    rule: Rule,
    work: Declaration<T>,
}

struct Queue<T> {
    value: Option<Arc<T>>,
    draining: bool,
    ended: bool,
    waiting: Vec<Waiting<T>>,
    next_id: u64,
}

impl<T> Queue<T> {
    fn push(&mut self, name: String, rule: Rule, work: Declaration<T>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        if matches!(rule, Rule::Event) {
            let events = self.waiting.iter().filter(|waiting| matches!(waiting.rule, Rule::Event)).count();
            if events >= WAITING_EVENT_CAPACITY {
                if let Some(oldest) = self.waiting.iter().position(|waiting| matches!(waiting.rule, Rule::Event)) {
                    self.waiting.remove(oldest);
                }
            }
        }
        self.waiting.push(Waiting { id, name, rule, work });
        id
    }

    fn supersede(&mut self, name: &str) {
        let group = format!("{name}/");
        self.waiting.retain(|waiting| !(matches!(waiting.rule, Rule::Latest) && (waiting.name == name || waiting.name.starts_with(&group))));
    }

    fn cancel_open(&mut self, name: &str) -> bool {
        let Some(open) = self.waiting.iter().rposition(|waiting| matches!(waiting.rule, Rule::Opens) && waiting.name == name) else { return false };
        self.waiting.remove(open);
        true
    }
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
        Arc::new(Deferred { queue: Mutex::new(Queue { value: None, draining: false, ended: false, waiting: Vec::new(), next_id: 0 }), outcome })
    }

    fn submit<P: Send + 'static>(&self, payload: P, work: impl FnOnce(&T, P) + Send + 'static, plan: impl FnOnce(&mut Queue<T>, &P) -> Plan) -> Option<u64> {
        let mut queue = locked(&self.queue);
        match &queue.value {
            Some(value) if !queue.draining => {
                let value = value.clone();
                drop(queue);
                work(&value, payload);
                None
            }
            _ if queue.ended => None,
            _ => match plan(&mut queue, &payload) {
                Plan::Push(name, rule) => Some(queue.push(name, rule, Box::new(move |value| work(value, payload)))),
                Plan::Drop => None,
            },
        }
    }

    pub fn run(&self, work: impl FnOnce(&T) + Send + 'static) {
        self.submit((), move |value, ()| work(value), |_, _| Plan::Push(String::new(), Rule::Event));
    }

    pub fn run_latest<P: Send + 'static>(&self, payload: P, name: fn(&P) -> String, work: impl FnOnce(&T, P) + Send + 'static) {
        self.submit(payload, work, |queue, payload| {
            let name = name(payload);
            queue.supersede(&name);
            Plan::Push(name, Rule::Latest)
        });
    }

    pub fn run_opening<P: Send + 'static>(&self, payload: P, name: fn(&P) -> String, work: impl FnOnce(&T, P) + Send + 'static) {
        self.submit(payload, work, |_, payload| Plan::Push(name(payload), Rule::Opens));
    }

    pub fn run_closing<P: Send + 'static>(&self, payload: P, name: fn(&P) -> String, work: impl FnOnce(&T, P) + Send + 'static) {
        self.submit(payload, work, |queue, payload| {
            let name = name(payload);
            if queue.cancel_open(&name) { Plan::Drop } else { Plan::Push(name, Rule::Closes) }
        });
    }

    pub fn run_resetting(&self, work: impl FnOnce(&T) + Send + 'static) {
        self.submit((), move |value, ()| work(value), |queue, _| {
            queue.waiting.clear();
            Plan::Push(String::new(), Rule::Event)
        });
    }

    pub fn is_pending(&self) -> bool {
        let queue = locked(&self.queue);
        !queue.ended && (queue.value.is_none() || queue.draining)
    }

    #[cfg(test)]
    pub fn queued(&self) -> usize {
        locked(&self.queue).waiting.len()
    }

    pub fn install(&self, value: Arc<T>) {
        let mut queue = locked(&self.queue);
        if queue.ended {
            return;
        }
        queue.value = Some(value.clone());
        queue.draining = true;
        loop {
            let waiting = std::mem::take(&mut queue.waiting);
            if waiting.is_empty() {
                queue.draining = false;
                self.outcome.send_replace(Some(Ok(value)));
                return;
            }
            drop(queue);
            for waiting in waiting {
                let ended = locked(&self.queue).ended;
                if ended {
                    break;
                }
                (waiting.work)(&value);
            }
            queue = locked(&self.queue);
            if queue.ended {
                return;
            }
        }
    }

    pub fn fail(&self, error: MediaError) {
        self.end(error);
    }

    pub fn close(&self) {
        self.end(MediaError::Unexpected { detail: "the media layer is closed".to_string() });
    }

    fn end(&self, error: MediaError) {
        let mut queue = locked(&self.queue);
        if queue.ended {
            return;
        }
        queue.ended = true;
        let value = queue.value.take();
        let waiting = std::mem::take(&mut queue.waiting);
        let settled = self.outcome.send_replace(Some(Err(error)));
        drop(queue);
        drop((value, waiting, settled));
    }

    pub async fn wait(&self) -> Result<Arc<T>, MediaError> {
        let mut outcome = self.outcome.subscribe();
        let settled = outcome.wait_for(Option::is_some).await.map_err(|error| MediaError::Unexpected { detail: error.to_string() })?;
        settled.clone().expect("a settled outcome holds a value")
    }

    pub fn hold(self: &Arc<Self>, subscribe: impl FnOnce(&T) -> Subscription + Send + 'static) -> Subscription {
        let held = Arc::new(Mutex::new(Held::default()));
        let fill = held.clone();
        let queued = self.submit(
            (),
            move |value, ()| {
                let mut held = locked(&fill);
                if !held.cancelled {
                    held.subscription = Some(subscribe(value));
                }
            },
            |_, _| Plan::Push(String::new(), Rule::Subscription),
        );
        let deferred = Arc::downgrade(self);
        Subscription::cancelling(move || {
            let mut held = locked(&held);
            held.cancelled = true;
            held.subscription.take();
            drop(held);
            if let (Some(id), Some(deferred)) = (queued, deferred.upgrade()) {
                locked(&deferred.queue).waiting.retain(|waiting| waiting.id != id);
            }
        })
    }
}
