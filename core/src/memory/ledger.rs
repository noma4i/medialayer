use super::degradation::{Step, StepPlan};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Owner {
    Slot(usize),
    ProxyBuffer,
    ImageDecode,
    Queue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    #[error("{owner:?} asked for {requested} bytes with {available} available")]
    Ceiling { owner: Owner, requested: u64, available: u64 },
    #[error("{owner:?} could not allocate {requested} bytes")]
    Allocation { owner: Owner, requested: u64 },
}

pub trait Freeing {
    fn free(&mut self, needed: u64) -> u64;
}

impl<F: FnMut(u64) -> u64> Freeing for F {
    fn free(&mut self, needed: u64) -> u64 {
        self(needed)
    }
}

#[derive(Debug)]
struct State {
    used: u64,
    step: Step,
    by_owner: HashMap<Owner, u64>,
}

#[derive(Debug)]
struct Inner {
    ceiling: AtomicU64,
    state: Mutex<State>,
    freed: Notify,
    waiting: AtomicUsize,
}

struct Waiting<'a>(&'a AtomicUsize);

impl<'a> Waiting<'a> {
    fn start(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Waiting(counter)
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

enum Between {
    Granted(Reservation),
    Refused(Refused),
    Later,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Debug, Clone)]
pub struct Ledger(Arc<Inner>);

impl Ledger {
    pub fn new(ceiling: u64) -> Self {
        Self(Arc::new(Inner {
            ceiling: AtomicU64::new(ceiling),
            state: Mutex::new(State { used: 0, step: Step::Normal, by_owner: HashMap::new() }),
            freed: Notify::new(),
            waiting: AtomicUsize::new(0),
        }))
    }

    pub fn unbounded() -> Self {
        Self::new(u64::MAX)
    }

    pub fn used(&self) -> u64 {
        self.0.lock().used
    }

    pub fn ceiling(&self) -> u64 {
        self.0.ceiling.load(Ordering::SeqCst)
    }

    pub fn set_ceiling(&self, ceiling: u64) {
        self.0.ceiling.store(ceiling, Ordering::SeqCst);
        self.0.freed.notify_waiters();
    }

    pub fn freed(&self) -> &Notify {
        &self.0.freed
    }

    pub fn waiting(&self) -> usize {
        self.0.waiting.load(Ordering::SeqCst)
    }

    pub fn used_by(&self, owner: Owner) -> u64 {
        self.0.lock().by_owner.get(&owner).copied().unwrap_or(0)
    }

    pub fn set_pressure_step(&self, step: Step) {
        self.0.lock().step = step;
        self.0.freed.notify_waiters();
    }

    pub fn reserve(&self, owner: Owner, bytes: u64) -> Result<Reservation, Refused> {
        let mut state = self.0.lock();
        let limit = self.admission_limit(state.step);
        let available = limit.saturating_sub(state.used);
        let used = state.used.checked_add(bytes).filter(|used| *used <= limit);
        let Some(used) = used else {
            return Err(Refused::Ceiling { owner, requested: bytes, available });
        };
        state.used = used;
        *state.by_owner.entry(owner).or_insert(0) += bytes;
        Ok(Reservation { ledger: self.clone(), owner, bytes })
    }

    pub async fn reserve_between(&self, owner: Owner, floor: u64, wanted: u64) -> Result<Reservation, Refused> {
        let floor = floor.min(wanted);
        loop {
            let freed = self.0.freed.notified();
            tokio::pin!(freed);
            freed.as_mut().enable();
            match self.reserve_at_least(owner, floor, wanted) {
                Between::Granted(reservation) => return Ok(reservation),
                Between::Refused(refused) => return Err(refused),
                Between::Later => {}
            }
            let _waiting = Waiting::start(&self.0.waiting);
            freed.await;
        }
    }

    fn reserve_at_least(&self, owner: Owner, floor: u64, wanted: u64) -> Between {
        let mut state = self.0.lock();
        let limit = self.admission_limit(state.step);
        if floor > limit {
            return Between::Refused(Refused::Ceiling { owner, requested: floor, available: limit });
        }
        let available = limit.saturating_sub(state.used);
        if available < floor {
            return Between::Later;
        }
        let granted = wanted.min(available);
        state.used += granted;
        *state.by_owner.entry(owner).or_insert(0) += granted;
        Between::Granted(Reservation { ledger: self.clone(), owner, bytes: granted })
    }

    pub fn reserve_or_free(&self, owner: Owner, bytes: u64, freeing: &mut impl Freeing) -> Result<Reservation, Refused> {
        match self.reserve(owner, bytes) {
            Err(Refused::Ceiling { available, .. }) => {
                freeing.free(bytes.saturating_sub(available));
                self.reserve(owner, bytes)
            }
            other => other,
        }
    }

    pub fn alloc(&self, owner: Owner, len: u64) -> Result<Buffer, Refused> {
        let reservation = self.reserve(owner, len)?;
        let mut data = Vec::new();
        let size = usize::try_from(len).ok().filter(|size| data.try_reserve_exact(*size).is_ok());
        let Some(size) = size else {
            return Err(Refused::Allocation { owner, requested: len });
        };
        data.resize(size, 0);
        Ok(Buffer { data: data.into_boxed_slice(), _reservation: reservation })
    }

    fn admission_limit(&self, step: Step) -> u64 {
        let percent = u128::from(StepPlan::for_step(step).admission_percent);
        (u128::from(self.ceiling()) * percent / 100) as u64
    }

    fn release(&self, owner: Owner, bytes: u64) {
        let mut state = self.0.lock();
        state.used = state.used.saturating_sub(bytes);
        if let Some(held) = state.by_owner.get_mut(&owner) {
            *held = held.saturating_sub(bytes);
            if *held == 0 {
                state.by_owner.remove(&owner);
            }
        }
        drop(state);
        self.0.freed.notify_waiters();
    }
}

#[derive(Debug)]
pub struct Reservation {
    ledger: Ledger,
    owner: Owner,
    bytes: u64,
}

impl Reservation {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn owner(&self) -> Owner {
        self.owner
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.ledger.release(self.owner, self.bytes);
    }
}

#[derive(Debug, Clone)]
pub struct Held(Arc<Reservation>);

impl Held {
    pub fn new(reservation: Reservation) -> Held {
        Held(Arc::new(reservation))
    }

    pub fn bytes(&self) -> u64 {
        self.0.bytes
    }

    pub fn downgrade(&self) -> std::sync::Weak<Reservation> {
        Arc::downgrade(&self.0)
    }

    pub fn holders(&self) -> usize {
        Arc::strong_count(&self.0)
    }
}

#[derive(Debug)]
pub struct Buffer {
    data: Box<[u8]>,
    _reservation: Reservation,
}

impl Buffer {
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }
}
