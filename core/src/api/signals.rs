use super::guard::AbortOnDrop;
use super::ports::{MemoryRelease, PoolControl, PrefetchControl, SlotDirectives, Suspendable};
use super::types::Pressure;
use crate::cache::Clock;
use crate::memory::{Degradation, Ledger};
use crate::pool::Directive;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::runtime::Handle;
use tokio::sync::Notify;
use tokio::task::JoinSet;

const QUEUE_CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signal {
    Memory(Pressure),
    Foreground,
    Background,
}

struct Queue {
    pending: Mutex<VecDeque<Signal>>,
    wake: Notify,
}

impl Queue {
    fn pending(&self) -> MutexGuard<'_, VecDeque<Signal>> {
        self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn push(&self, signal: Signal) {
        let mut pending = self.pending();
        if pending.len() >= QUEUE_CAPACITY {
            collapse(&mut pending);
        }
        pending.push_back(signal);
        drop(pending);
        self.wake.notify_one();
    }

    fn pop(&self) -> Option<Signal> {
        self.pending().pop_front()
    }
}

fn collapse(pending: &mut VecDeque<Signal>) {
    let worst = pending.iter().filter_map(|signal| match signal {
        Signal::Memory(level) => Some(*level),
        _ => None,
    }).fold(None, |worst: Option<Pressure>, level| match (worst, level) {
        (Some(Pressure::Critical), _) | (_, Pressure::Critical) => Some(Pressure::Critical),
        _ => Some(Pressure::Moderate),
    });
    let last_lifecycle = pending.iter().rposition(|signal| !matches!(signal, Signal::Memory(_)));
    let first_memory = pending.iter().position(|signal| matches!(signal, Signal::Memory(_)));
    let kept: VecDeque<Signal> = pending
        .iter()
        .enumerate()
        .filter_map(|(index, signal)| match signal {
            Signal::Memory(_) => (Some(index) == first_memory).then(|| Signal::Memory(worst.unwrap_or(Pressure::Moderate))),
            lifecycle => (Some(index) == last_lifecycle).then_some(*lifecycle),
        })
        .collect();
    *pending = kept;
}

struct Reaction {
    pool: Arc<dyn PoolControl>,
    prefetch: Arc<dyn PrefetchControl>,
    directives: Arc<dyn SlotDirectives>,
    release: Arc<dyn MemoryRelease>,
    suspendable: Vec<Arc<dyn Suspendable>>,
    ledger: Ledger,
    clock: Arc<dyn Clock>,
    degradation: Mutex<(Degradation, Option<crate::memory::Step>)>,
    blocking: Arc<Mutex<JoinSet<()>>>,
}

pub struct Signals {
    queue: Arc<Queue>,
    worker: Mutex<Option<AbortOnDrop>>,
    blocking: Arc<Mutex<JoinSet<()>>>,
}

pub struct SignalPorts {
    pub pool: Arc<dyn PoolControl>,
    pub prefetch: Arc<dyn PrefetchControl>,
    pub directives: Arc<dyn SlotDirectives>,
    pub release: Arc<dyn MemoryRelease>,
    pub suspendable: Vec<Arc<dyn Suspendable>>,
    pub ledger: Ledger,
    pub clock: Arc<dyn Clock>,
}

impl Signals {
    pub fn new(handle: &Handle, ports: SignalPorts) -> Signals {
        let SignalPorts { pool, prefetch, directives, release, suspendable, ledger, clock } = ports;
        let queue = Arc::new(Queue { pending: Mutex::new(VecDeque::new()), wake: Notify::new() });
        let blocking = Arc::new(Mutex::new(JoinSet::new()));
        let reaction = Arc::new(Reaction { pool, prefetch, directives, release, suspendable, ledger, clock, degradation: Mutex::new((Degradation::default(), None)), blocking: blocking.clone() });
        let inbox = queue.clone();
        let worker = super::guard::restart(handle, "signals", move || {
            let inbox = inbox.clone();
            let reaction = reaction.clone();
            async move {
            let mut decay = None;
            loop {
                while let Some(signal) = inbox.pop() {
                    if matches!(signal, Signal::Memory(_)) {
                        decay = Some(tokio::time::Instant::now() + std::time::Duration::from_secs(10));
                    }
                    reaction.apply(signal);
                }
                tokio::select! {
                    _ = inbox.wake.notified() => {},
                    _ = reaction.ledger.released().notified() => {
                        reaction.lower_step(None);
                    },
                    _ = async {
                        match decay {
                            Some(deadline) => tokio::time::sleep_until(deadline).await,
                            None => std::future::pending().await,
                        }
                    } => {
                        decay = None;
                        reaction.lower_step(None);
                    },
                }
            }
            }
        });
        Signals { queue, worker: Mutex::new(Some(AbortOnDrop::new(worker))), blocking }
    }

    /// Stops the worker and waits for the platform cache drops it already started: none runs after this returns.
    pub async fn close(&self) {
        let worker = self.worker.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        if let Some(mut worker) = worker {
            worker.abort();
            let _ = worker.join().await;
        }
        let mut started = std::mem::take(&mut *self.blocking.lock().unwrap_or_else(|poisoned| poisoned.into_inner()));
        while started.join_next().await.is_some() {}
    }

    pub fn memory_warning(&self, level: Pressure) {
        self.queue.push(Signal::Memory(level));
    }

    pub fn foreground(&self) {
        self.queue.push(Signal::Foreground);
    }

    pub fn background(&self) {
        self.queue.push(Signal::Background);
    }
}

impl Reaction {
    fn apply(&self, signal: Signal) {
        match signal {
            Signal::Memory(level) => {
                self.lower_step(Some(level));
                let directives = self.pool.release_under(self.clock.now_ms(), level.into());
                self.dispatch(directives);
            }
            Signal::Background => {
                self.suspendable.iter().for_each(|work| work.suspend());
                self.dispatch(self.pool.background());
            }
            Signal::Foreground => {
                self.lower_step(None);
                self.pool.foreground();
                self.suspendable.iter().for_each(|work| work.resume());
            }
        }
    }

    fn dispatch(&self, directives: Vec<Directive>) {
        let (stops, rest): (Vec<Directive>, Vec<Directive>) =
            directives.into_iter().partition(|directive| matches!(directive, Directive::StopPrefetch));
        if !stops.is_empty() {
            self.prefetch.stop();
        }
        let (drops, rest): (Vec<Directive>, Vec<Directive>) =
            rest.into_iter().partition(|directive| matches!(directive, Directive::DropMemoryCache));
        if !drops.is_empty() {
            let release = self.release.clone();
            self.blocking.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).spawn_blocking(move || release.drop_caches());
        }
        if !rest.is_empty() {
            self.directives.apply(rest);
        }
    }

    fn lower_step(&self, level: Option<Pressure>) {
        let mut degradation = self.degradation.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let step = degradation.0.for_pressure(self.ledger.used(), self.ledger.ceiling(), level.map(Into::into));
        if level.is_none() && degradation.1 == Some(step) {
            return;
        }
        if level.is_none() {
            crate::trace!("heal", "capacity step-restored {step:?}");
        }
        self.pool.set_step(step);
        degradation.1 = Some(step);
    }
}
