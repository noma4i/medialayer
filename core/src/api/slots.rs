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
}

pub struct SlotRunner {
    players: Option<Arc<dyn PlayerSlotFactory>>,
    handle: Handle,
    lanes: Arc<Mutex<Lanes>>,
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
        SlotRunner { players, handle, lanes: Arc::new(Mutex::new(Lanes::default())) }
    }

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
        let lane = lanes.live.entry(slot).or_insert_with(|| {
            let shared = Arc::new(Shared { queue: Queue::default(), life: Mutex::new(SlotLife::new()) });
            let task = self.handle.spawn(run_lane(players, slot, shared.clone(), Arc::downgrade(&self.lanes)));
            Lane { shared, _task: AbortOnDrop::new(task), #[cfg(test)] taken_task: None }
        });
        lane.shared.queue.push(call, held);
    }

    pub fn dispose(&self, slot: usize) {
        let mut lanes = self.lanes();
        if let Some(lane) = lanes.live.remove(&slot) {
            lane.shared.queue.dispose();
            lanes.closing.push(lane);
        }
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
                perform(&players, slot, &shared, call, held, revision).await;
                shared.queue.finish();
            }
            Next::Dispose => {
                let player = shared.life().dispose();
                if let Some(player) = player {
                    player.dispose().await;
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

async fn perform(players: &Arc<dyn PlayerSlotFactory>, slot: usize, shared: &Shared, call: SlotCall, held: Option<Held>, revision: u64) {
    if call == SlotCall::Unload {
        let player = shared.life().release();
        if let Some(player) = player {
            player.apply(SlotCall::Unload).await;
            shared.life().acked();
        }
        return;
    }
    let started = shared.life().start(held);
    let current = match started {
        Start::Ready(player) => player,
        Start::Create => {
            let created = players.create(slot as u32).await;
            let player = shared.life().created(created.player);
            if shared.queue.is_replaced_since(revision) {
                return;
            }
            player
        }
        Start::Gone => return,
    };
    let is_load = matches!(call, SlotCall::Load { .. });
    if matches!(call, SlotCall::Seek { .. }) {
        tokio::select! {
            () = current.apply(call) => {}
            () = shared.queue.source_replaced_since(revision) => {}
        }
    } else {
        current.apply(call).await;
    }
    if is_load {
        shared.life().applied();
    }
}
