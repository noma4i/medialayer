use super::slots::SlotCall;
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};
use tokio::sync::Notify;

pub const LANE_CAPACITY: usize = 16;

#[derive(Default)]
struct Pending {
    calls: VecDeque<SlotCall>,
    in_flight: bool,
    source_revision: u64,
}

#[derive(Default)]
pub struct Queue {
    pending: Mutex<Pending>,
    pub wake: Notify,
    source_replaced: Notify,
}

impl Queue {
    pub fn push(&self, call: SlotCall) {
        let mut state = self.pending();
        let replaces_source = matches!(call, SlotCall::Load { .. } | SlotCall::Unload);
        if replaces_source {
            state.source_revision += 1;
            let pending = &mut state.calls;
            let sound = pending.iter().rfind(|queued| matches!(queued, SlotCall::Mute | SlotCall::Unmute)).cloned();
            let release = pending.contains(&SlotCall::Unload).then_some(SlotCall::Unload).filter(|_| matches!(call, SlotCall::Load { .. }));
            pending.clear();
            pending.extend(sound);
            pending.extend(release);
        }
        let pending = &mut state.calls;
        pending.push_back(call);
        if pending.len() > LANE_CAPACITY {
            *pending = fold(pending);
        }
        drop(state);
        self.wake.notify_one();
        if replaces_source {
            self.source_replaced.notify_waiters();
        }
    }

    pub fn begin_batch(&self) -> bool {
        let mut state = self.pending();
        state.in_flight = !state.calls.is_empty();
        state.in_flight
    }

    pub fn pop(&self) -> Option<(SlotCall, u64)> {
        let mut state = self.pending();
        let call = state.calls.pop_front();
        state.in_flight = call.is_some();
        call.map(|call| (call, state.source_revision))
    }

    pub async fn source_replaced_since(&self, revision: u64) {
        loop {
            let replaced = self.source_replaced.notified();
            tokio::pin!(replaced);
            replaced.as_mut().enable();
            if self.pending().source_revision != revision {
                return;
            }
            replaced.await;
        }
    }

    pub fn finish(&self) {
        self.pending().in_flight = false;
    }

    pub fn discard(&self) {
        self.pending().calls.clear();
    }

    pub fn len(&self) -> usize {
        self.pending().calls.len()
    }

    pub fn is_idle(&self) -> bool {
        let state = self.pending();
        state.calls.is_empty() && !state.in_flight
    }

    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn fold(calls: &VecDeque<SlotCall>) -> VecDeque<SlotCall> {
    let mut sound = None;
    let mut source = None;
    let mut release = None;
    let mut play = None;
    let mut view = None;
    let mut seek = None;
    let mut rate = None;
    let mut readahead = None;
    for call in calls {
        match call {
            SlotCall::Mute | SlotCall::Unmute => sound = Some(call.clone()),
            SlotCall::Load { .. } | SlotCall::Unload => {
                release = (*call == SlotCall::Unload || release.is_some()).then_some(SlotCall::Unload);
                source = Some(call.clone());
                play = None;
                view = None;
                seek = None;
                rate = None;
                readahead = None;
            }
            SlotCall::Play | SlotCall::Pause => play = Some(call.clone()),
            SlotCall::Attach { .. } | SlotCall::Detach => view = Some(call.clone()),
            SlotCall::Seek { .. } => seek = Some(call.clone()),
            SlotCall::Rate { .. } => rate = Some(call.clone()),
            SlotCall::Readahead { .. } => readahead = Some(call.clone()),
            SlotCall::Preroll => {}
        }
    }
    let release = release.filter(|_| source != Some(SlotCall::Unload));
    [sound, release, source, readahead, view, rate, seek, play].into_iter().flatten().collect()
}

#[cfg(test)]
mod lane_tests;
