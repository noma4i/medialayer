use crate::fetch::{BodyStream, FetchError};
use bytes::Bytes;
use futures_util::Stream;
use std::cell::Cell;
use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};
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
    playback: VecDeque<oneshot::Sender<Permit>>,
    prefetch: VecDeque<oneshot::Sender<Permit>>,
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

struct HeldBody {
    body: BodyStream,
    permit: Option<Permit>,
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
            CLOSING_INLINE.with(|inline| inline.set(true));
            drop(body);
            CLOSING_INLINE.with(|inline| inline.set(false));
            drop(permit);
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn_blocking(close);
            }
            Err(_) => close(),
        }
    }
}

impl Permit {
    pub fn hold(self, body: BodyStream) -> BodyStream {
        Box::pin(HeldBody { body, permit: Some(self) })
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
                playback: VecDeque::new(),
                prefetch: VecDeque::new(),
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
    }

    pub fn in_use(&self) -> usize {
        lock(&self.state).in_use
    }

    pub async fn acquire(&self, priority: Priority) -> Result<Permit, QueueFull> {
        let receiver = {
            let mut state = lock(&self.state);
            state.drop_abandoned();
            if state.can_take_now(priority) {
                state.in_use += 1;
                return Ok(Permit { state: Some(self.state.clone()) });
            }
            if priority.is_prefetch() && state.prefetch.len() >= state.max_waiting {
                return Err(QueueFull);
            }
            let (sender, receiver) = oneshot::channel();
            crate::trace!("slots", "wait {priority:?} in_use={} playback={} prefetch={}", state.in_use, state.playback.len(), state.prefetch.len());
            if priority.is_prefetch() {
                state.prefetch.push_back(sender);
            } else {
                state.playback.push_back(sender);
            }
            receiver
        };
        match receiver.await {
            Ok(permit) => Ok(permit),
            Err(_) => Err(QueueFull),
        }
    }
}

impl State {
    fn drop_abandoned(&mut self) {
        self.playback.retain(|waiter| !waiter.is_closed());
        self.prefetch.retain(|waiter| !waiter.is_closed());
    }

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
                self.playback.pop_front()
            } else if self.in_use < self.prefetch_limit() && self.playback.is_empty() {
                self.prefetch.pop_front()
            } else {
                None
            };
            let Some(sender) = waiter else { return };
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
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}
