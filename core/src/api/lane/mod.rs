use super::slots::SlotCall;
use crate::memory::Held;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use tokio::sync::Notify;

pub const LANE_CAPACITY: usize = 16;

#[derive(Debug, Clone)]
pub struct Queued {
    pub call: SlotCall,
    pub held: Option<Held>,
    marker: Option<(u64, u64, tokio::sync::watch::Sender<(u64, std::collections::BTreeMap<u64, (u64, u64)>)>)>,
}

impl Queued {
    fn bare(call: SlotCall) -> Queued {
        Queued { call, held: None, marker: None }
    }
}

#[derive(Debug)]
pub enum Next {
    Call { call: SlotCall, held: Option<Held>, revision: u64 },
    Dispose,
}

#[derive(Default)]
struct Pending {
    calls: VecDeque<Queued>,
    in_flight: bool,
    source_revision: u64,
    is_disposing: bool,
    progress: Option<(u64, tokio::sync::watch::Sender<(u64, std::collections::BTreeMap<u64, (u64, u64)>)>)>,
}

#[derive(Default)]
pub struct Queue {
    pending: Mutex<Pending>,
    pub wake: Notify,
    source_replaced: Notify,
    settled: Notify,
    is_settled: AtomicBool,
}

impl Queue {
    pub fn push(&self, call: SlotCall, held: Option<Held>) {
        let mut state = self.pending();
        if state.is_disposing {
            return;
        }
        let replaces_source = matches!(call, SlotCall::Load { .. } | SlotCall::Unload);
        let mut stale = VecDeque::new();
        if replaces_source {
            state.source_revision += 1;
            let pending = &mut state.calls;
            let protected = pending.iter().rposition(|queued| queued.marker.is_some()).map_or(0, |index| index + 1);
            let mut suffix = pending.split_off(protected);
            let pending = &mut suffix;
            let sound = pending.iter().rfind(|queued| matches!(queued.call, SlotCall::Mute | SlotCall::Unmute)).map(|queued| Queued::bare(queued.call.clone()));
            let release = pending.iter().any(|queued| queued.call == SlotCall::Unload).then(|| Queued::bare(SlotCall::Unload)).filter(|_| matches!(call, SlotCall::Load { .. }));
            stale = std::mem::take(pending);
            pending.extend(sound);
            pending.extend(release);
            state.calls.extend(suffix);
        }
        let pending = &mut state.calls;
        if call == SlotCall::Pause {
            let protected = pending.iter().rposition(|queued| queued.marker.is_some()).map_or(0, |index| index + 1);
            let mut suffix = pending.split_off(protected);
            suffix.retain(|queued| queued.call != SlotCall::Play);
            pending.extend(suffix);
        }
        pending.push_back(Queued { call, held, marker: None });
        if pending.len() > LANE_CAPACITY {
            let protected = pending.iter().rposition(|queued| queued.marker.is_some()).map_or(0, |index| index + 1);
            let suffix = pending.split_off(protected);
            let folded = fold(&suffix);
            stale.extend(suffix);
            pending.extend(folded);
            if let Some(marker_index) = protected.checked_sub(1) {
                let suffix = pending.split_off(marker_index + 1);
                if let Some(marker) = pending.pop_back() {
                    let prefix = std::mem::take(pending);
                    pending.extend(fold(&prefix));
                    pending.push_back(marker);
                    stale.extend(prefix);
                }
                pending.extend(suffix);
            }
        }
        drop(state);
        drop(stale);
        self.wake.notify_one();
        if replaces_source {
            self.source_replaced.notify_waiters();
        }
    }

    pub fn marker(&self, id: u64, sequence: u64, progress: tokio::sync::watch::Sender<(u64, std::collections::BTreeMap<u64, (u64, u64)>)>) {
        let mut state = self.pending();
        if !state.is_disposing {
            state.calls.retain(|queued| queued.marker.is_none());
            progress.send_modify(|state| state.1.entry(id).or_default().0 = sequence);
            state.progress = Some((id, progress.clone()));
            state.calls.push_back(Queued { call: SlotCall::Pause, held: None, marker: Some((id, sequence, progress)) });
        }
        drop(state);
        self.wake.notify_one();
    }

    pub fn dispose(&self) {
        let mut state = self.pending();
        if state.is_disposing {
            return;
        }
        state.is_disposing = true;
        state.source_revision += 1;
        let protected = state.calls.iter().rposition(|queued| queued.marker.is_some()).map_or(0, |index| index + 1);
        let stale = state.calls.split_off(protected);
        drop(state);
        drop(stale);
        self.wake.notify_one();
        self.source_replaced.notify_waiters();
    }

    pub fn take(&self) -> Option<Next> {
        let mut state = self.pending();
        let revision = state.source_revision;
        while let Some(Queued { call, held, marker }) = state.calls.pop_front() {
            if let Some((id, sequence, progress)) = marker {
                progress.send_modify(|state| {
                    if let Some((_, acked)) = state.1.get_mut(&id) { *acked = (*acked).max(sequence); }
                });
                continue;
            }
            state.in_flight = true;
            return Some(Next::Call { call, held, revision });
        }
        if state.is_disposing && !self.is_settled.load(Ordering::SeqCst) && !state.in_flight {
            state.in_flight = true;
            return Some(Next::Dispose);
        }
        None
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

    pub fn is_replaced_since(&self, revision: u64) -> bool {
        self.pending().source_revision != revision
    }

    pub fn finish(&self) {
        self.pending().in_flight = false;
        #[cfg(test)]
        crate::test_signal::fire();
    }

    pub fn settle(&self) {
        self.is_settled.store(true, Ordering::SeqCst);
        let mut state = self.pending();
        state.in_flight = false;
        if let Some((id, progress)) = state.progress.take() { progress.send_modify(|state| { state.1.remove(&id); }); }
        drop(state);
        self.settled.notify_waiters();
        #[cfg(test)]
        crate::test_signal::fire();
    }

    pub async fn settled(&self) {
        loop {
            let settled = self.settled.notified();
            tokio::pin!(settled);
            settled.as_mut().enable();
            if self.is_settled.load(Ordering::SeqCst) {
                return;
            }
            settled.await;
        }
    }

    pub fn is_disposing(&self) -> bool {
        self.pending().is_disposing
    }

    pub fn len(&self) -> usize {
        self.pending().calls.len()
    }

    pub fn is_idle(&self) -> bool {
        let state = self.pending();
        state.calls.is_empty() && !state.in_flight && (!state.is_disposing || self.is_settled.load(Ordering::SeqCst))
    }

    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn fold(calls: &VecDeque<Queued>) -> VecDeque<Queued> {
    let mut sound = None;
    let mut source: Option<Queued> = None;
    let mut release = None;
    let mut play = None;
    let mut view = None;
    let mut seek = None;
    let mut rate = None;
    let mut looping = None;
    let mut readahead = None;
    for queued in calls {
        match &queued.call {
            SlotCall::Mute | SlotCall::Unmute => sound = Some(queued.clone()),
            SlotCall::Load { .. } | SlotCall::Unload => {
                release = (queued.call == SlotCall::Unload || release.is_some()).then(|| Queued::bare(SlotCall::Unload));
                source = Some(queued.clone());
                play = None;
                view = None;
                seek = None;
                rate = None;
                looping = None;
                readahead = None;
            }
            SlotCall::Play | SlotCall::Pause => play = Some(queued.clone()),
            SlotCall::Attach { .. } | SlotCall::Detach => view = Some(queued.clone()),
            SlotCall::Seek { .. } => seek = Some(queued.clone()),
            SlotCall::Rate { .. } => rate = Some(queued.clone()),
            SlotCall::Looping { .. } => looping = Some(queued.clone()),
            SlotCall::Readahead { .. } => readahead = Some(queued.clone()),
            SlotCall::Preroll => {}
        }
    }
    let release = release.filter(|_| source.as_ref().map(|queued| &queued.call) != Some(&SlotCall::Unload));
    [sound, release, source, readahead, view, rate, looping, seek, play].into_iter().flatten().collect()
}

#[cfg(test)]
mod lane_tests;
