use super::source::PlaybackRoute;
use crate::pool::{Command, Retention};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RetainLevel {
    Hot,
    Warm,
    Cold,
}

impl From<Retention> for RetainLevel {
    fn from(level: Retention) -> Self {
        match level {
            Retention::Hot => RetainLevel::Hot,
            Retention::Warm => RetainLevel::Warm,
            Retention::Cold => RetainLevel::Cold,
        }
    }
}

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
    fn retain(&self, level: RetainLevel);
    fn sample(&self) -> SlotSample;
}

#[uniffi::export(with_foreign)]
pub trait PlayerSlotFactory: Send + Sync {
    fn create(&self, slot: u32) -> Arc<dyn PlayerSlot>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotCall {
    Load { key: String, route: PlaybackRoute },
    Unload,
    Preroll,
    Play,
    Pause,
    Mute,
    Unmute,
    Retain(RetainLevel),
}

impl SlotCall {
    pub fn from_command(command: Command, route: Option<PlaybackRoute>) -> (usize, SlotCall) {
        match command {
            Command::Load { slot, key } => {
                let route = route.unwrap_or_else(|| PlaybackRoute { url: key.clone(), headers: HashMap::new(), generation: None });
                (slot, SlotCall::Load { key, route })
            }
            Command::Unload { slot } => (slot, SlotCall::Unload),
            Command::Preroll { slot } => (slot, SlotCall::Preroll),
            Command::Play { slot } => (slot, SlotCall::Play),
            Command::Pause { slot } => (slot, SlotCall::Pause),
            Command::Mute { slot } => (slot, SlotCall::Mute),
            Command::Unmute { slot } => (slot, SlotCall::Unmute),
            Command::Retain { slot, level } => (slot, SlotCall::Retain(level.into())),
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
            SlotCall::Retain(level) => player.retain(level),
        }
    }
}

type Player = Arc<Mutex<Option<Arc<dyn PlayerSlot>>>>;

struct Lane {
    queue: mpsc::UnboundedSender<SlotCall>,
    player: Player,
}

pub struct SlotRunner {
    factory: Option<Arc<dyn PlayerSlotFactory>>,
    handle: Handle,
    lanes: Mutex<HashMap<usize, Lane>>,
}

impl SlotRunner {
    pub fn new(handle: Handle, factory: Option<Arc<dyn PlayerSlotFactory>>) -> SlotRunner {
        SlotRunner { factory, handle, lanes: Mutex::new(HashMap::new()) }
    }

    pub fn send(&self, slot: usize, call: SlotCall) {
        let Some(factory) = self.factory.clone() else { return };
        let mut lanes = self.lanes();
        let lane = lanes.entry(slot).or_insert_with(|| {
            let (queue, inbox) = mpsc::unbounded_channel();
            let player: Player = Arc::new(Mutex::new(None));
            self.handle.spawn(run_lane(factory, slot, inbox, player.clone()));
            Lane { queue, player }
        });
        let _ = lane.queue.send(call);
    }

    pub async fn sample(&self, slot: usize) -> Option<SlotSample> {
        let player = self.lanes().get(&slot)?.player.clone();
        let player = player.lock().unwrap_or_else(PoisonError::into_inner).clone()?;
        tokio::task::spawn_blocking(move || player.sample()).await.ok()
    }

    fn lanes(&self) -> MutexGuard<'_, HashMap<usize, Lane>> {
        self.lanes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

async fn run_lane(
    factory: Arc<dyn PlayerSlotFactory>,
    slot: usize,
    mut inbox: mpsc::UnboundedReceiver<SlotCall>,
    player: Player,
) {
    while let Some(call) = inbox.recv().await {
        let factory = factory.clone();
        let cell = player.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let mut held = cell.lock().unwrap_or_else(PoisonError::into_inner);
            let current = held.get_or_insert_with(|| factory.create(slot as u32)).clone();
            drop(held);
            call.apply(current.as_ref());
        })
        .await;
    }
}
