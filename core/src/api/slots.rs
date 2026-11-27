use super::guard::AbortOnDrop;
use super::lane::{Next, Queue};
use super::slot_life::{Player, SlotLife, Start};
use crate::memory::Held;
use super::source::PlaybackRoute;
use crate::pool::Command;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use tokio::runtime::Handle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct SlotSample {
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub is_looping: bool,
    pub view_attached: bool,
    pub frame_rendered: bool,
}

#[uniffi::export(with_foreign)]
#[async_trait]
pub trait PlayerSlot: Send + Sync {
    async fn apply(&self, command: SlotCall);
    async fn dispose(&self);
    fn sample(&self) -> SlotSample;
}

#[derive(uniffi::Record)]
pub struct CreatedPlayer {
    pub player: Arc<dyn PlayerSlot>,
}

#[uniffi::export(with_foreign)]
#[async_trait]
pub trait PlayerSlotFactory: Send + Sync {
    async fn create(&self, slot: u32) -> CreatedPlayer;
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum SlotCall {
    Load { key: String, route: PlaybackRoute, generation: u64 },
    Unload,
    Preroll,
    Play,
    Pause,
    Mute,
    Unmute,
    Attach { view_token: u64 },
    Detach,
    Seek { position_ms: u64 },
    Rate { rate: f32 },
    Looping { looping: bool },
    Readahead { ms: u64 },
}

impl SlotCall {
    pub fn from_command(command: Command) -> Option<(usize, SlotCall)> {
        Some(match command {
            Command::Unload { slot } => (slot, SlotCall::Unload),
            Command::Preroll { slot } => (slot, SlotCall::Preroll),
            Command::Play { slot } => (slot, SlotCall::Play),
            Command::Pause { slot } => (slot, SlotCall::Pause),
            Command::Mute { slot } => (slot, SlotCall::Mute),
            Command::Unmute { slot } => (slot, SlotCall::Unmute),
            Command::Load { .. } | Command::Retain { .. } | Command::Dispose { .. } => return None,
        })
    }

    pub fn label(&self) -> String {
        match self {
            SlotCall::Load { key, route, .. } => format!("Load {} via {}", crate::trace::tail(key), crate::trace::tail(&route.url)),
            other => format!("{other:?}"),
        }
    }
}

struct Shared {
    queue: Queue,
    life: Mutex<SlotLife>,
    pause_id: u64,
    timeout: std::time::Duration,
    on_timeout: Arc<Mutex<Option<Arc<dyn Fn(usize, u64) + Send + Sync>>>>,
    on_load: Arc<Mutex<Option<Arc<dyn Fn(usize, u64, &str, u64) -> bool + Send + Sync>>>>,
}

impl Shared {
    fn life(&self) -> MutexGuard<'_, SlotLife> {
        self.life.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn player(&self) -> Option<Player> {
        self.life().player()
    }
}

struct Lane {
    slot: usize,
    shared: Arc<Shared>,
    _task: AbortOnDrop,
    #[cfg(test)]
    taken_task: Option<AbortTask>,
}

#[cfg(test)]
struct AbortTask(tokio::task::AbortHandle);

#[cfg(test)]
impl Drop for AbortTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Default)]
struct Lanes {
    live: HashMap<usize, Lane>,
    closing: Vec<Lane>,
    is_closed: bool,
    pause_barrier: Option<(u64, tokio::sync::watch::Sender<(u64, std::collections::BTreeMap<u64, (u64, u64)>)>)>,
}

pub struct SlotRunner {
    players: Option<Arc<dyn PlayerSlotFactory>>,
    handle: Handle,
    lanes: Arc<Mutex<Lanes>>,
    timeout: Mutex<std::time::Duration>,
    on_timeout: Arc<Mutex<Option<Arc<dyn Fn(usize, u64) + Send + Sync>>>>,
    on_load: Arc<Mutex<Option<Arc<dyn Fn(usize, u64, &str, u64) -> bool + Send + Sync>>>>,
}

impl SlotRunner {
    #[cfg(test)]
    pub fn observe_task(&self, slot: usize) -> Option<AbortOnDrop> {
        let mut lanes = self.lanes();
        let lane = lanes.live.get_mut(&slot)?;
        let replacement = AbortOnDrop::new(self.handle.spawn(async {}));
        let task = std::mem::replace(&mut lane._task, replacement);
        lane.taken_task = Some(AbortTask(task.abort_handle()));
        Some(task)
    }

    pub fn new(handle: Handle, players: Option<Arc<dyn PlayerSlotFactory>>) -> SlotRunner {
        SlotRunner { players, handle, lanes: Arc::new(Mutex::new(Lanes::default())), timeout: Mutex::new(std::time::Duration::from_secs(8)), on_timeout: Arc::default(), on_load: Arc::default() }
    }

    pub fn set_timeout(&self, duration: std::time::Duration) { *self.timeout.lock().unwrap_or_else(PoisonError::into_inner) = duration; }

    pub fn on_timeout(&self, callback: Arc<dyn Fn(usize, u64) + Send + Sync>) { *self.on_timeout.lock().unwrap_or_else(PoisonError::into_inner) = Some(callback); }

    pub fn on_load(&self, callback: Arc<dyn Fn(usize, u64, &str, u64) -> bool + Send + Sync>) { *self.on_load.lock().unwrap_or_else(PoisonError::into_inner) = Some(callback); }

    pub fn send(&self, slot: usize, call: SlotCall) {
        self.send_held(slot, call, None);
    }

    pub fn send_held(&self, slot: usize, call: SlotCall, held: Option<Held>) {
        let Some(players) = self.players.clone() else { return };
        crate::trace!("slot", "send {slot} {}", call.label());
        let mut lanes = self.lanes();
        if lanes.is_closed {
            return;
        }
        if lanes.live.get(&slot).is_some_and(|lane| lane.shared.queue.is_disposing()) {
            let old = lanes.live.remove(&slot);
            lanes.closing.extend(old);
        }
        let barrier = if call == SlotCall::Pause { lanes.pause_barrier.clone() } else { None };
        let previous = lanes.closing.iter().rev().find(|lane| lane.slot == slot).map(|lane| lane.shared.clone());
        let lane = lanes.live.entry(slot).or_insert_with(|| {
            static NEXT_PAUSE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let shared = Arc::new(Shared { queue: Queue::default(), life: Mutex::new(SlotLife::new()), pause_id: NEXT_PAUSE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed), timeout: *self.timeout.lock().unwrap_or_else(PoisonError::into_inner), on_timeout: self.on_timeout.clone(), on_load: self.on_load.clone() });
            let next = shared.clone();
            let lanes = Arc::downgrade(&self.lanes);
            let task = self.handle.spawn(async move {
                if let Some(previous) = previous { previous.queue.settled().await; }
                run_lane(players, slot, next, lanes).await;
            });
            Lane { slot, shared, _task: AbortOnDrop::new(task), #[cfg(test)] taken_task: None }
        });
        lane.shared.queue.push(call, held);
        if let Some((sequence, progress)) = barrier { lane.shared.queue.marker(lane.shared.pause_id, sequence, progress); }
    }

    pub(crate) fn begin_pause_barrier(&self, sequence: u64, progress: tokio::sync::watch::Sender<(u64, std::collections::BTreeMap<u64, (u64, u64)>)>) {
        self.lanes().pause_barrier = Some((sequence, progress));
    }

    pub(crate) fn end_pause_barrier(&self) {
        if let Some((sequence, progress)) = self.lanes().pause_barrier.take() {
            progress.send_modify(|state| state.0 = state.0.max(sequence));
        }
    }

    pub fn dispose(&self, slot: usize) {
        let mut lanes = self.lanes();
        if let Some(lane) = lanes.live.remove(&slot) {
            lane.shared.queue.dispose();
            lanes.closing.push(lane);
        }
    }

    pub fn dispose_current(&self, slot: usize, generation: u64) -> bool {
        let mut lanes = self.lanes();
        if !lanes.live.get(&slot).is_some_and(|lane| lane.shared.pause_id == generation) { return false; }
        let lane = lanes.live.remove(&slot).unwrap();
        lane.shared.queue.dispose();
        lanes.closing.push(lane);
        true
    }

    pub fn dispose_all(&self) {
        let mut lanes = self.lanes();
        lanes.is_closed = true;
        let live: Vec<Lane> = lanes.live.drain().map(|(_, lane)| lane).collect();
        for lane in &live {
            lane.shared.queue.dispose();
        }
        lanes.closing.extend(live);
    }

    pub async fn disposed(&self) {
        loop {
            let waiting: Vec<Arc<Shared>> = self.lanes().closing.iter().map(|lane| lane.shared.clone()).collect();
            if waiting.is_empty() {
                return;
            }
            for shared in waiting {
                shared.queue.settled().await;
            }
        }
    }

    pub fn is_idle(&self) -> bool {
        let lanes = self.lanes();
        lanes.live.values().chain(lanes.closing.iter()).all(|lane| lane.shared.queue.is_idle())
    }

    pub fn queued(&self, slot: usize) -> usize {
        self.lanes().live.get(&slot).map_or(0, |lane| lane.shared.queue.len())
    }

    pub fn live_slots(&self) -> usize {
        let lanes = self.lanes();
        lanes.live.len() + lanes.closing.len()
    }

    pub fn sample_now(&self, slot: usize) -> Option<SlotSample> {
        let shared = self.lanes().live.get(&slot)?.shared.clone();
        let player = shared.player()?;
        Some(player.sample())
    }

    pub async fn sample(&self, slot: usize) -> Option<SlotSample> {
        let shared = self.lanes().live.get(&slot)?.shared.clone();
        let player = shared.player()?;
        tokio::task::spawn_blocking(move || player.sample()).await.ok()
    }

    fn lanes(&self) -> MutexGuard<'_, Lanes> {
        self.lanes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

async fn run_lane(players: Arc<dyn PlayerSlotFactory>, slot: usize, shared: Arc<Shared>, lanes: Weak<Mutex<Lanes>>) {
    loop {
        let Some(next) = shared.queue.take() else {
            shared.queue.wake.notified().await;
            continue;
        };
        match next {
            Next::Call { call, held, revision } => {
                if let SlotCall::Load { key, generation, .. } = &call {
                    let callback = shared.on_load.lock().unwrap_or_else(PoisonError::into_inner).clone();
                    if callback.is_some_and(|callback| callback(slot, shared.pause_id, key, *generation)) {
                        shared.queue.finish();
                        continue;
                    }
                }
                let timed_out = perform(&players, slot, &shared, call, held, revision).await;
                shared.queue.finish();
                if timed_out {
                    let player = shared.life().player();
                    if let Some(player) = player { let _ = tokio::time::timeout(shared.timeout, player.dispose()).await; }
                    shared.life().abandon();
                    let callback = shared.on_timeout.lock().unwrap_or_else(PoisonError::into_inner).clone();
                    if let Some(callback) = callback { callback(slot, shared.pause_id); }
                }
            }
            Next::Dispose => {
                let player = shared.life().dispose();
                if let Some(player) = player {
                    let _ = tokio::time::timeout(shared.timeout, player.dispose()).await;
                }
                shared.life().acked();
                shared.queue.settle();
                if let Some(lanes) = lanes.upgrade() {
                    let retired = {
                        let mut lanes = lanes.lock().unwrap_or_else(PoisonError::into_inner);
                        lanes.closing.iter().position(|lane| Arc::ptr_eq(&lane.shared, &shared)).map(|position| lanes.closing.remove(position))
                    };
                    drop(retired);
                }
                return;
            }
        }
    }
}

async fn perform(players: &Arc<dyn PlayerSlotFactory>, slot: usize, shared: &Shared, call: SlotCall, held: Option<Held>, revision: u64) -> bool {
    if call == SlotCall::Unload {
        let player = shared.life().release();
        if let Some(player) = player {
            if tokio::time::timeout(shared.timeout, player.apply(SlotCall::Unload)).await.is_err() { return true; }
            shared.life().acked();
        }
        return false;
    }
    let started = shared.life().start(held);
    let current = match started {
        Start::Ready(player) => player,
        Start::Create => {
            let Ok(created) = tokio::time::timeout(shared.timeout, players.create(slot as u32)).await else { return true };
            let player = shared.life().created(created.player);
            if shared.queue.is_replaced_since(revision) {
                return false;
            }
            player
        }
        Start::Gone => return false,
    };
    let is_load = matches!(call, SlotCall::Load { .. });
    if matches!(call, SlotCall::Seek { .. }) {
        tokio::select! {
            () = current.apply(call) => {}
            () = shared.queue.source_replaced_since(revision) => {}
        }
    } else {
        if tokio::time::timeout(shared.timeout, current.apply(call)).await.is_err() { return true; }
    }
    if is_load {
        shared.life().applied();
    }
    false
}
