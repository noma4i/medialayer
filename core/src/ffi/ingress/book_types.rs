use super::command::{Observer, PlaceInput};
use super::queue::{InputQueue, Reserve};
use super::readiness::PageReadiness;
use crate::api::{MediaItem, Subscription};
use tokio::runtime::Handle;
use super::super::assembled::Assembled;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use super::book_keys::*;

pub(super) struct Place {
    pub(super) item: MediaItem,
    pub(super) input: PlaceInput,
    pub(super) declared: u64,
    pub(super) intent: u64,
    pub(super) _opened: Reserve,
    pub(super) updated: Reserve,
}

pub(super) struct VideoWindowState {
    pub(super) sources: Arc<Vec<Option<MediaItem>>>,
    pub(super) active: u32,
    pub(super) is_active: bool,
    pub(super) intent: u64,
    pub(super) replaced: u64,
    pub(super) ready: PageReadiness,
    pub(super) _opened: Reserve,
    pub(super) held: Reserve,
}

impl VideoWindowState {
    pub(super) fn declares_source(&self) -> bool {
        self.sources.iter().any(Option::is_some)
    }
}

pub(super) struct ImageWindowState {
    pub(super) scoped: String,
    pub(super) pages: Arc<Vec<Vec<MediaItem>>>,
    pub(super) active: u32,
    pub(super) is_active: bool,
    pub(super) ready: PageReadiness,
    pub(super) _opened: Reserve,
    pub(super) held: Reserve,
}

pub(super) enum Window {
    Video(VideoWindowState),
    Image(ImageWindowState),
}

pub(super) struct Observed {
    pub(super) _subscription: Subscription,
    pub(super) listener: Observer,
    pub(super) _reserve: Reserve,
}

/// Executor-owned state: aggregation per key, windows, subscriptions. Single-threaded, no locks.
pub(crate) struct Book {
    pub(super) assembled: Arc<Assembled>,
    pub(super) queue: Arc<InputQueue>,
    pub(super) handle: Handle,
    pub(super) seq: u64,
    pub(super) places: HashMap<u64, Place>,
    pub(super) keys: HashMap<String, KeyOwners>,
    pub(super) windows: HashMap<u64, Window>,
    pub(super) video_order: BTreeMap<u64, u64>,
    pub(super) video_applied: Option<(u64, u64)>,
    pub(super) subscriptions: HashMap<u64, Observed>,
    pub(super) suspensions: HashMap<u64, Reserve>,
    pub(super) in_call: bool,
    pub(super) paused: bool,
    pub(super) declarations: HashMap<u64, Reserve>,
}
