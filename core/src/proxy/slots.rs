use crate::fetch::{BodyStream, FetchError};
use bytes::Bytes;
use futures_util::Stream;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::Duration;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Playback,
    Prefetch,
    Image,
    ImagePrefetch,
}

impl Priority {
    pub fn is_prefetch(self) -> bool {
        matches!(self, Priority::Prefetch | Priority::ImagePrefetch)
    }

    pub fn promoted(self) -> Priority {
        match self {
            Priority::Prefetch => Priority::Playback,
            Priority::ImagePrefetch => Priority::Image,
            other => other,
        }
    }

    pub fn is_image(self) -> bool {
        matches!(self, Priority::Image | Priority::ImagePrefetch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFull;

struct State {
    max: usize,
    max_waiting: usize,
    reserved: usize,
    in_use: usize,
    next: u64,
    playback: BTreeMap<u64, oneshot::Sender<Permit>>,
    prefetch: BTreeMap<u64, oneshot::Sender<Permit>>,
}

struct Waiting {
    state: Arc<Mutex<State>>,
    seq: u64,
    prefetch: bool,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let mut state = lock(&self.state);
        let queue = if self.prefetch { &mut state.prefetch } else { &mut state.playback };
        if queue.remove(&self.seq).is_some() {
            #[cfg(test)]
            crate::test_signal::fire();
            state.hand_out(&self.state);
        }
    }
}

#[derive(Clone)]
pub struct PrioritySlots {
    state: Arc<Mutex<State>>,
}

pub struct Permit {
    state: Option<Arc<Mutex<State>>>,
}

thread_local! {
    static CLOSING_INLINE: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn closing_inline() -> bool {
    CLOSING_INLINE.with(Cell::get)
}

struct InlineClose;

impl InlineClose {
    fn enter() -> InlineClose {
        CLOSING_INLINE.with(|inline| inline.set(true));
        InlineClose
    }
}

impl Drop for InlineClose {
    fn drop(&mut self) {
        CLOSING_INLINE.with(|inline| inline.set(false));
    }
}

struct HeldBody {
    body: BodyStream,
    permit: Option<Permit>,
}

struct IdleState {
    body: Option<BodyStream>,
    last_poll: tokio::time::Instant,
    waker: Option<Waker>,
}

struct IdleBody {
    state: Arc<Mutex<IdleState>>,
    _task: crate::api::guard::AbortOnDrop,
}

impl Stream for IdleBody {
    type Item = Result<Bytes, FetchError>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut state = lock(&self.state);
        state.last_poll = tokio::time::Instant::now();
        state.waker = Some(context.waker().clone());
        match state.body.as_mut() {
            Some(body) => body.as_mut().poll_next(context),
            None => Poll::Ready(None),
        }
    }
}

impl Drop for IdleBody {
    fn drop(&mut self) {
        let body = lock(&self.state).body.take();
        drop(body);
    }
}

impl Stream for HeldBody {
    type Item = Result<Bytes, FetchError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.body.as_mut().poll_next(context)
    }
}

impl Drop for HeldBody {
    fn drop(&mut self) {
        let body = std::mem::replace(&mut self.body, Box::pin(futures_util::stream::empty()));
        let permit = self.permit.take();
        let close = move || {
            let _inline = InlineClose::enter();
            drop(body);
            drop(_inline);
            drop(permit);
        };
        crate::detached::release_once(close);
    }
}

impl Permit {
    pub fn hold(self, body: BodyStream) -> BodyStream {
        Box::pin(HeldBody { body, permit: Some(self) })
    }

    pub fn hold_idle(self, body: BodyStream, idle: Duration) -> BodyStream {
        let state = Arc::new(Mutex::new(IdleState { body: Some(self.hold(body)), last_poll: tokio::time::Instant::now(), waker: None }));
        let task_state = state.clone();
        let task = tokio::spawn(async move {
            loop {
                let deadline = lock(&task_state).last_poll + idle;
                tokio::time::sleep_until(deadline).await;
                let mut state = lock(&task_state);
                if tokio::time::Instant::now() < state.last_poll + idle {
                    continue;
                }
                let body = state.body.take();
                let waker = state.waker.take();
                drop(state);
                drop(body);
                if let Some(waker) = waker { waker.wake(); }
                crate::trace!("heal", "capacity body-dropped");
                break;
            }
        });
        Box::pin(IdleBody { state, _task: crate::api::guard::AbortOnDrop::new(task) })
    }
}

impl PrioritySlots {
    pub fn new(max: usize, reserved: usize, max_waiting: usize) -> Self {
        let max = max.max(1);
        PrioritySlots {
            state: Arc::new(Mutex::new(State {
                max,
                max_waiting,
                reserved,
                in_use: 0,
                next: 0,
                playback: BTreeMap::new(),
                prefetch: BTreeMap::new(),
            })),
        }
    }

    pub fn set_max(&self, max: usize) {
        let mut state = lock(&self.state);
        state.max = max.max(1);
        state.hand_out(&self.state);
    }

    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        let state = lock(&self.state);
        state.playback.len() + state.prefetch.len()
    }

    #[cfg(test)]
    pub fn lose_waiters(&self) {
        let mut state = lock(&self.state);
        state.playback.clear();
        state.prefetch.clear();
        crate::test_signal::fire();
    }

    pub fn in_use(&self) -> usize {
        lock(&self.state).in_use
    }

    pub async fn acquire(&self, priority: Priority) -> Result<Permit, QueueFull> {
        let (receiver, _waiting) = {
            let mut state = lock(&self.state);
            if state.can_take_now(priority) {
                state.in_use += 1;
                return Ok(Permit { state: Some(self.state.clone()) });
            }
            let prefetch = priority.is_prefetch();
            if prefetch && state.prefetch.len() >= state.max_waiting {
                return Err(QueueFull);
            }
            let (sender, receiver) = oneshot::channel();
            crate::trace!("slots", "wait {priority:?} in_use={} playback={} prefetch={}", state.in_use, state.playback.len(), state.prefetch.len());
            let seq = state.next;
            state.next += 1;
            if prefetch {
                state.prefetch.insert(seq, sender);
            } else {
                state.playback.insert(seq, sender);
            }
            #[cfg(test)]
            crate::test_signal::fire();
            (receiver, Waiting { state: self.state.clone(), seq, prefetch })
        };
        match receiver.await {
            Ok(permit) => Ok(permit),
            Err(_) => Err(QueueFull),
        }
    }
}

impl State {
    fn prefetch_limit(&self) -> usize {
        self.max - self.reserved.min(self.max - 1)
    }

    fn can_take_now(&self, priority: Priority) -> bool {
        if priority.is_prefetch() {
            self.in_use < self.prefetch_limit() && self.playback.is_empty() && self.prefetch.is_empty()
        } else {
            self.in_use < self.max && self.playback.is_empty()
        }
    }

    fn hand_out(&mut self, shared: &Arc<Mutex<State>>) {
        loop {
            let waiter = if self.in_use < self.max && !self.playback.is_empty() {
                self.playback.pop_first().map(|(_, sender)| sender)
            } else if self.in_use < self.prefetch_limit() && self.playback.is_empty() {
                self.prefetch.pop_first().map(|(_, sender)| sender)
            } else {
                None
            };
            let Some(sender) = waiter else { return };
            #[cfg(test)]
            crate::test_signal::fire();
            self.in_use += 1;
            if let Err(mut returned) = sender.send(Permit { state: Some(shared.clone()) }) {
                returned.state = None;
                self.in_use -= 1;
            }
        }
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let Some(shared) = self.state.take() else { return };
        let mut state = lock(&shared);
        state.in_use -= 1;
        state.hand_out(&shared);
        #[cfg(test)]
        crate::test_signal::fire();
    }
}

fn lock<T>(state: &Mutex<T>) -> MutexGuard<'_, T> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}
