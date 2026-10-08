use super::slots::SlotCall;
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, PoisonError};
use tokio::sync::Notify;

pub const LANE_CAPACITY: usize = 16;

#[derive(Default)]
struct Pending {
    calls: VecDeque<SlotCall>,
    in_flight: bool,
}

#[derive(Default)]
pub struct Queue {
    pending: Mutex<Pending>,
    pub wake: Notify,
}

impl Queue {
    pub fn push(&self, call: SlotCall) {
        let mut state = self.pending();
        let pending = &mut state.calls;
        if matches!(call, SlotCall::Load { .. } | SlotCall::Unload) {
            let sound = pending.iter().rfind(|queued| matches!(queued, SlotCall::Mute | SlotCall::Unmute)).cloned();
            let release = pending.contains(&SlotCall::Unload).then_some(SlotCall::Unload).filter(|_| matches!(call, SlotCall::Load { .. }));
            pending.clear();
            pending.extend(sound);
            pending.extend(release);
        }
        pending.push_back(call);
        if pending.len() > LANE_CAPACITY {
            *pending = fold(pending);
        }
        drop(state);
        self.wake.notify_one();
    }

    pub fn pop(&self) -> Option<SlotCall> {
        let mut state = self.pending();
        let call = state.calls.pop_front();
        state.in_flight = call.is_some();
        call
    }

    pub fn finish(&self) {
        self.pending().in_flight = false;
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
    let mut retain = None;
    let mut play = None;
    let mut view = None;
    let mut seek = None;
    let mut rate = None;
    for call in calls {
        match call {
            SlotCall::Mute | SlotCall::Unmute => sound = Some(call.clone()),
            SlotCall::Load { .. } | SlotCall::Unload => {
                release = (*call == SlotCall::Unload || release.is_some()).then_some(SlotCall::Unload);
                source = Some(call.clone());
                retain = None;
                play = None;
                view = None;
                seek = None;
                rate = None;
            }
            SlotCall::Retain(_) => retain = Some(call.clone()),
            SlotCall::Play | SlotCall::Pause => play = Some(call.clone()),
            SlotCall::Attach { .. } | SlotCall::Detach => view = Some(call.clone()),
            SlotCall::Seek { .. } => seek = Some(call.clone()),
            SlotCall::Rate { .. } => rate = Some(call.clone()),
            SlotCall::Preroll => {}
        }
    }
    let release = release.filter(|_| source != Some(SlotCall::Unload));
    [sound, release, source, retain, view, rate, seek, play].into_iter().flatten().collect()
}

#[cfg(test)]
mod lane_tests;
