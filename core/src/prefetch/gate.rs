use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::oneshot;

pub(super) struct Gate {
    limit: AtomicUsize,
    state: Mutex<Queue>,
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
        Arc::new(Gate { limit: AtomicUsize::new(limit), state: Mutex::new(Queue::default()) })
    }

    pub fn set_limit(self: &Arc<Gate>, limit: usize) {
        self.limit.store(limit, Ordering::SeqCst);
        let mut queue = self.queue();
        self.grant(&mut queue);
    }

    pub async fn acquire(self: &Arc<Gate>, held: Option<LoadPermit>, distance: &Arc<AtomicUsize>, chunk: usize) -> LoadPermit {
        let receiver = {
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
            receiver
        };
        receiver.await.expect("the gate outlives its waiters")
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
