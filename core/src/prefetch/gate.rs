use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::oneshot;

pub(super) struct Gate {
    limit: AtomicUsize,
    state: Mutex<Queue>,
    #[cfg(test)]
    queued: tokio::sync::Notify,
}

#[derive(Default)]
struct Queue {
    running: usize,
    next: u64,
    waiters: Vec<Waiter>,
}

struct Waiter {
    distance: Arc<AtomicUsize>,
    chunk: usize,
    seq: u64,
    wake: oneshot::Sender<LoadPermit>,
}

struct Pending {
    gate: Arc<Gate>,
    seq: u64,
}

impl Drop for Pending {
    fn drop(&mut self) {
        let mut queue = self.gate.queue();
        if let Some(index) = queue.waiters.iter().position(|waiter| waiter.seq == self.seq) {
            queue.waiters.swap_remove(index);
        }
    }
}

pub(super) struct LoadPermit(Option<Arc<Gate>>);

impl Drop for LoadPermit {
    fn drop(&mut self) {
        let Some(gate) = self.0.take() else { return };
        let mut queue = gate.queue();
        queue.running -= 1;
        gate.grant(&mut queue);
    }
}

impl Gate {
    pub fn new(limit: usize) -> Arc<Gate> {
        Arc::new(Gate { limit: AtomicUsize::new(limit), state: Mutex::new(Queue::default()), #[cfg(test)] queued: tokio::sync::Notify::new() })
    }

    pub fn set_limit(self: &Arc<Gate>, limit: usize) {
        self.limit.store(limit, Ordering::SeqCst);
        let mut queue = self.queue();
        self.grant(&mut queue);
    }

    pub async fn acquire(self: &Arc<Gate>, held: Option<LoadPermit>, distance: &Arc<AtomicUsize>, chunk: usize) -> LoadPermit {
        let (receiver, _pending) = {
            let mut queue = self.queue();
            if let Some(mut permit) = held {
                permit.0 = None;
                queue.running -= 1;
            }
            let (wake, receiver) = oneshot::channel();
            let seq = queue.next;
            queue.next += 1;
            queue.waiters.push(Waiter { distance: distance.clone(), chunk, seq, wake });
            self.grant(&mut queue);
            #[cfg(test)]
            self.queued.notify_one();
            (receiver, Pending { gate: self.clone(), seq })
        };
        receiver.await.expect("the gate outlives its waiters")
    }

    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.queue().waiters.len()
    }

    #[cfg(test)]
    pub async fn wait_queued(&self, count: usize) {
        while self.waiting() < count {
            self.queued.notified().await;
        }
    }

    #[cfg(test)]
    pub fn running(&self) -> usize {
        self.queue().running
    }

    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn grant(self: &Arc<Gate>, queue: &mut Queue) {
        while queue.running < self.limit.load(Ordering::SeqCst) {
            let Some(index) = queue
                .waiters
                .iter()
                .enumerate()
                .min_by_key(|(_, waiter)| (waiter.distance.load(Ordering::SeqCst), waiter.chunk, waiter.seq))
                .map(|(index, _)| index)
            else {
                return;
            };
            let waiter = queue.waiters.swap_remove(index);
            queue.running += 1;
            if let Err(mut permit) = waiter.wake.send(LoadPermit(Some(self.clone()))) {
                permit.0 = None;
                queue.running -= 1;
            }
        }
    }
}
