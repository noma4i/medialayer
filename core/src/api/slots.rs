use super::guard::AbortOnDrop;
use super::lane::{Queue, LANE_CAPACITY};
use super::source::PlaybackRoute;
use crate::pool::Command;
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
pub trait PlayerSlot: Send + Sync {
    fn load(&self, key: String, route: PlaybackRoute);
    fn unload(&self);
    fn preroll(&self);
    fn play(&self);
    fn pause(&self);
    fn mute(&self);
    fn unmute(&self);
    fn attach(&self, view_token: u64);
    fn detach(&self);
    fn seek(&self, position_ms: u64);
    fn set_rate(&self, rate: f32);
    fn set_readahead(&self, ms: u64);
    fn sample(&self) -> SlotSample;
}

#[uniffi::export(with_foreign)]
pub trait PlayerSlotFactory: Send + Sync {
    fn create(&self, slot: u32) -> Arc<dyn PlayerSlot>;
}

#[derive(Debug, Clone, PartialEq)]
pub enum SlotCall {
    Load { key: String, route: PlaybackRoute },
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
    pub fn from_command(command: Command, route: Option<PlaybackRoute>) -> Option<(usize, SlotCall)> {
        Some(match command {
            Command::Load { slot, key } => {
                let route = route.unwrap_or_else(|| PlaybackRoute { url: key.clone(), headers: HashMap::new(), generation: None, hls: false, looping: true });
                (slot, SlotCall::Load { key, route })
            }
            Command::Unload { slot } => (slot, SlotCall::Unload),
            Command::Preroll { slot } => (slot, SlotCall::Preroll),
            Command::Play { slot } => (slot, SlotCall::Play),
            Command::Pause { slot } => (slot, SlotCall::Pause),
            Command::Mute { slot } => (slot, SlotCall::Mute),
            Command::Unmute { slot } => (slot, SlotCall::Unmute),
            Command::Retain { .. } => return None,
        })
    }

    pub fn label(&self) -> String {
        match self {
            SlotCall::Load { key, route } => format!("Load {} via {}", crate::trace::tail(key), crate::trace::tail(&route.url)),
            other => format!("{other:?}"),
        }
    }

    fn apply(self, player: &dyn PlayerSlot) {
        match self {
            SlotCall::Load { key, route } => player.load(key, route),
            SlotCall::Unload => player.unload(),
            SlotCall::Preroll => player.preroll(),
            SlotCall::Play => player.play(),
            SlotCall::Pause => player.pause(),
            SlotCall::Mute => player.mute(),
            SlotCall::Unmute => player.unmute(),
            SlotCall::Attach { view_token } => player.attach(view_token),
            SlotCall::Detach => player.detach(),
            SlotCall::Seek { position_ms } => player.seek(position_ms),
            SlotCall::Rate { rate } => player.set_rate(rate),
            SlotCall::Readahead { ms } => player.set_readahead(ms),
        }
    }
}

type Player = Arc<OnceLock<Arc<dyn PlayerSlot>>>;

struct Lane {
    queue: Arc<Queue>,
    player: Player,
    _task: AbortOnDrop,
    #[cfg(test)]
    blocking_calls: Arc<std::sync::atomic::AtomicUsize>,
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
            #[cfg(test)]
            let blocking_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let task = self.handle.spawn(run_lane(players, slot, queue.clone(), player.clone(), #[cfg(test)] blocking_calls.clone()));
            Lane { queue, player, _task: AbortOnDrop::new(task), #[cfg(test)] blocking_calls }
        });
        lane.queue.push(call);
    }

    pub fn is_idle(&self) -> bool {
        self.lanes().values().all(|lane| lane.queue.is_idle())
    }

    pub fn queued(&self, slot: usize) -> usize {
        self.lanes().get(&slot).map_or(0, |lane| lane.queue.len())
    }

    #[cfg(test)]
    pub fn blocking_calls(&self, slot: usize) -> usize {
        self.lanes().get(&slot).map_or(0, |lane| lane.blocking_calls.load(std::sync::atomic::Ordering::Relaxed))
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

async fn run_lane(players: Arc<dyn PlayerSlotFactory>, slot: usize, queue: Arc<Queue>, player: Player, #[cfg(test)] blocking_calls: Arc<std::sync::atomic::AtomicUsize>) {
    loop {
        if !queue.begin_batch() {
            queue.wake.notified().await;
            continue;
        }
        let players = players.clone();
        let cell = player.clone();
        let pending = queue.clone();
        #[cfg(test)]
        blocking_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _ = tokio::task::spawn_blocking(move || {
            let current = cell.get_or_init(|| players.create(slot as u32));
            for _ in 0..LANE_CAPACITY {
                let Some(call) = pending.pop() else { break };
                call.apply(current.as_ref());
            }
        })
        .await;
        queue.finish();
    }
}
