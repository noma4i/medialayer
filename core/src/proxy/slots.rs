use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Playback,
    Prefetch,
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

impl PrioritySlots {
    pub fn new(max: usize, reserved: usize, max_waiting: usize) -> Self {
        let max = max.max(1);
        PrioritySlots {
            state: Arc::new(Mutex::new(State {
                max,
                max_waiting,
                reserved: reserved.min(max - 1),
                in_use: 0,
                playback: VecDeque::new(),
                prefetch: VecDeque::new(),
            })),
        }
    }

    #[cfg(test)]
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
            if priority == Priority::Prefetch && state.prefetch.len() >= state.max_waiting {
                return Err(QueueFull);
            }
            let (sender, receiver) = oneshot::channel();
            match priority {
                Priority::Playback => state.playback.push_back(sender),
                Priority::Prefetch => state.prefetch.push_back(sender),
            }
            receiver
        };
        match receiver.await {
            Ok(permit) => Ok(permit),
            Err(_) => std::future::pending().await,
        }
    }
}

impl State {
    fn drop_abandoned(&mut self) {
        self.playback.retain(|waiter| !waiter.is_closed());
        self.prefetch.retain(|waiter| !waiter.is_closed());
    }

    fn prefetch_limit(&self) -> usize {
        self.max - self.reserved
    }

    fn can_take_now(&self, priority: Priority) -> bool {
        match priority {
            Priority::Playback => self.in_use < self.max && self.playback.is_empty(),
            Priority::Prefetch => {
                self.in_use < self.prefetch_limit() && self.playback.is_empty() && self.prefetch.is_empty()
            }
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
