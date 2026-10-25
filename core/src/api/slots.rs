use super::guard::AbortOnDrop;
use super::lane::Queue;
use super::source::PlaybackRoute;
use crate::pool::Command;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
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
            Command::Load { .. } | Command::Retain { .. } => return None,
        })
    }

    pub fn label(&self) -> String {
        match self {
            SlotCall::Load { key, route, .. } => format!("Load {} via {}", crate::trace::tail(key), crate::trace::tail(&route.url)),
            other => format!("{other:?}"),
        }
    }
}

type Player = Arc<OnceLock<Arc<dyn PlayerSlot>>>;

struct Lane {
    queue: Arc<Queue>,
    player: Player,
    _task: AbortOnDrop,
}

impl Drop for Lane {
    fn drop(&mut self) {
        self.queue.discard();
    }
}

pub struct SlotRunner {
    players: Option<Arc<dyn PlayerSlotFactory>>,
    handle: Handle,
    lanes: Mutex<HashMap<usize, Lane>>,
}

impl SlotRunner {
    pub fn new(handle: Handle, players: Option<Arc<dyn PlayerSlotFactory>>) -> SlotRunner {
        SlotRunner { players, handle, lanes: Mutex::new(HashMap::new()) }
    }

    pub fn send(&self, slot: usize, call: SlotCall) {
        let Some(players) = self.players.clone() else { return };
        crate::trace!("slot", "send {slot} {}", call.label());
        let mut lanes = self.lanes();
        let lane = lanes.entry(slot).or_insert_with(|| {
            let queue = Arc::new(Queue::default());
            let player: Player = Arc::new(OnceLock::new());
            let task = self.handle.spawn(run_lane(players, slot, queue.clone(), player.clone()));
            Lane { queue, player, _task: AbortOnDrop::new(task) }
        });
        lane.queue.push(call);
    }

    pub fn is_idle(&self) -> bool {
        self.lanes().values().all(|lane| lane.queue.is_idle())
    }

    pub fn queued(&self, slot: usize) -> usize {
        self.lanes().get(&slot).map_or(0, |lane| lane.queue.len())
    }

    pub fn sample_now(&self, slot: usize) -> Option<SlotSample> {
        let player = self.lanes().get(&slot)?.player.clone();
        let player = player.get()?.clone();
        Some(player.sample())
    }

    pub async fn sample(&self, slot: usize) -> Option<SlotSample> {
        let player = self.lanes().get(&slot)?.player.clone();
        let player = player.get()?.clone();
        tokio::task::spawn_blocking(move || player.sample()).await.ok()
    }

    fn lanes(&self) -> MutexGuard<'_, HashMap<usize, Lane>> {
        self.lanes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

async fn run_lane(players: Arc<dyn PlayerSlotFactory>, slot: usize, queue: Arc<Queue>, player: Player) {
    loop {
        if !queue.begin_batch() {
            queue.wake.notified().await;
            continue;
        }
        let current = match player.get() {
            Some(current) => current.clone(),
            None => {
                let created = players.create(slot as u32).await;
                player.get_or_init(|| created.player).clone()
            }
        };
        while let Some((call, revision)) = queue.pop() {
            if matches!(call, SlotCall::Seek { .. }) {
                tokio::select! {
                    () = current.apply(call) => {}
                    () = queue.source_replaced_since(revision) => {}
                }
            } else {
                current.apply(call).await;
            }
        }
        queue.finish();
    }
}
