use super::weight::{self, Bytes};
use crate::analysis::{MediaTone, ToneError};
use crate::api::{AbortOnDrop, Placeholder, MediaError, MediaItem, MediaState, PlaybackRoute, Pressure, TonePreparation};
use crate::pool::PoolStats;
use crate::image_size::ImageSize;
use crate::proxy::ImageFetchResult;
use super::super::layer::{StateListener, StatsListener, ToneListener, WindowReadyListener};
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::Arc;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct PlaceInput {
    pub visible: bool,
    pub playing: bool,
    pub group: Option<String>,
    pub peeking: bool,
    pub muted: Option<bool>,
    pub rate: Option<f32>,
    pub pinned: bool,
    pub view_token: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnerKind {
    Place,
    Window,
    Subscription,
    Suspension,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WindowKind {
    Video,
    Image(String),
}

pub(crate) enum Observer {
    State(Arc<dyn StateListener>),
    Stats(Arc<dyn StatsListener>),
    Tone(Arc<dyn ToneListener>),
}

impl Observer {
    pub fn failed(&self, error: MediaError) {
        match self {
            Observer::State(listener) => listener.failed(error),
            Observer::Stats(listener) => listener.failed(error),
            Observer::Tone(listener) => listener.failed(error),
        }
    }
}

pub(crate) enum Subscribing {
    State { key: String, listener: Arc<dyn StateListener> },
    Stats { listener: Arc<dyn StatsListener> },
    Tone { item: MediaItem, band: f32, listener: Arc<dyn ToneListener> },
}

pub(crate) enum PlayerEvent {
    Ready { slot: u32, generation: u64 },
    VisualReady { slot: u32, generation: u64 },
    Stalled { slot: u32, generation: u64, position_ms: u64 },
    Failed { slot: u32, generation: u64, status: i32, detail: String },
    Ended { slot: u32, generation: u64 },
}

/// A request whose logical part the executor fixes at its barrier.
pub(crate) enum Query {
    PlayUrl(MediaItem),
    Refresh(MediaItem, PlaybackRoute),
    Pool,
    Cached(MediaItem),
    Tone(MediaItem, f32),
    Pixels(MediaItem),
    Image { url: String, headers: HashMap<String, String> },
    ImageSize { url: String, headers: HashMap<String, String> },
    Placeholder(MediaItem, bool),
}

pub(crate) enum Finished {
    Cached(bool),
    Tone(Result<Option<MediaTone>, ToneError>),
    Pixels(bool),
    Image(Result<ImageFetchResult, MediaError>),
    ImageSize(Result<Option<ImageSize>, MediaError>),
    Placeholder(Placeholder),
}

pub(crate) enum Answer {
    Route(PlaybackRoute),
    Pool(PoolStats),
    Placeholder(Placeholder),
    Running(AbortOnDrop<Finished>),
    Failed(MediaError),
}

pub(crate) type Reply = oneshot::Sender<Answer>;

pub(crate) enum Declared {
    PlaceOpen { owner: u64, item: MediaItem },
    PlaceUpdate { owner: u64, input: PlaceInput },
    PlaceSeek { owner: u64, fraction: f64 },
    KeySeek { key: String, fraction: f64 },
    PlaceState { owner: u64, state: MediaState },
    WindowOpen { owner: u64, kind: WindowKind },
    WindowActive { owner: u64, is_active: bool },
    VideoWindow { owner: u64, active: u32, sources: Option<Arc<Vec<Option<MediaItem>>>> },
    ImageWindow { owner: u64, active: u32, pages: Option<Arc<Vec<Vec<MediaItem>>>> },
    WindowReady { owner: u64, listener: Arc<dyn WindowReadyListener> },
    Suspend { owner: u64, groups: Vec<String> },
    Subscribe { owner: u64, subscribing: Subscribing },
    Player(PlayerEvent),
    Lifecycle { foreground: bool },
    Memory(Pressure),
    Preparation(Arc<dyn TonePreparation>),
    MutedAll(bool),
    ReleaseAll,
    /// Closes an owner at its place in the arrival order; it takes no admission place.
    Release { owner: u64, kind: OwnerKind },
    Query { query: Query, reply: Reply },
}

/// Commands with the same fold key may replace each other until the next barrier.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Fold {
    Place(u64),
    State(u64),
    KeySeek(String),
    Window(u64),
    WindowActive(u64),
    Lifecycle,
    Memory,
    Preparation,
    MutedAll,
}

/// What a fold did to the retained weight of the queued entry.
pub(crate) enum Merge {
    /// The queued payload stays and the weight of the newer command is not added.
    Kept,
    /// The newer command is now the entry and its weight replaces the old one.
    Replaced,
}

const SLOT: u64 = size_of::<Declared>() as u64;

impl Query {
    pub fn weight(&self) -> Bytes {
        match self {
            Query::PlayUrl(item) | Query::Cached(item) | Query::Pixels(item) | Query::Tone(item, _) | Query::Placeholder(item, _) => weight::item(item),
            Query::Refresh(item, route) => weight::sum([weight::item(item), weight::string(&route.url), weight::headers(&route.headers)]),
            Query::Pool => Some(0),
            Query::Image { url, headers } | Query::ImageSize { url, headers } => weight::sum([weight::string(url), weight::headers(headers)]),
        }
    }
}

impl Declared {
    pub fn fold(&self) -> Option<Fold> {
        Some(match self {
            Declared::PlaceUpdate { owner, .. } => Fold::Place(*owner),
            Declared::PlaceState { owner, .. } => Fold::State(*owner),
            Declared::KeySeek { key, .. } => Fold::KeySeek(key.clone()),
            Declared::VideoWindow { owner, .. } | Declared::ImageWindow { owner, .. } => Fold::Window(*owner),
            Declared::WindowActive { owner, .. } => Fold::WindowActive(*owner),
            Declared::Lifecycle { .. } => Fold::Lifecycle,
            Declared::Memory(_) => Fold::Memory,
            Declared::Preparation(_) => Fold::Preparation,
            Declared::MutedAll(_) => Fold::MutedAll,
            _ => return None,
        })
    }

    pub fn owner(&self) -> Option<u64> {
        match self {
            Declared::PlaceOpen { owner, .. }
            | Declared::PlaceUpdate { owner, .. }
            | Declared::PlaceSeek { owner, .. }
            | Declared::PlaceState { owner, .. }
            | Declared::WindowOpen { owner, .. }
            | Declared::WindowActive { owner, .. }
            | Declared::VideoWindow { owner, .. }
            | Declared::ImageWindow { owner, .. }
            | Declared::WindowReady { owner, .. }
            | Declared::Suspend { owner, .. }
            | Declared::Subscribe { owner, .. } => Some(*owner),
            _ => None,
        }
    }

    /// An event or a query closes the fold epoch: later updates cannot replace earlier entries.
    pub fn is_barrier(&self) -> bool {
        self.fold().is_none()
    }

    /// Retained bytes by backing capacity: the slot, strings, headers, vectors and `None` entries. `None` on overflow.
    pub fn weight(&self) -> Bytes {
        let own = match self {
            Declared::PlaceOpen { item, .. } => weight::item_heap(item),
            Declared::PlaceUpdate { input, .. } => input.group.as_ref().map_or(Some(0), weight::string),
            // The fold slot holds a second copy of the key.
            Declared::KeySeek { key, .. } => weight::sum([weight::string(key), weight::string(key)]),
            Declared::WindowOpen { kind: WindowKind::Image(name), .. } => weight::string(name),
            Declared::VideoWindow { sources: Some(sources), .. } => weight::sum([weight::sources(sources), Some(16 + size_of::<Vec<Option<MediaItem>>>() as u64)]),
            Declared::ImageWindow { pages: Some(pages), .. } => weight::sum([weight::pages(pages), Some(16 + size_of::<Vec<Vec<MediaItem>>>() as u64)]),
            Declared::Suspend { groups, .. } => weight::sum(std::iter::once(weight::slots::<String>(groups.capacity())).chain(groups.iter().map(weight::string))),
            Declared::Subscribe { subscribing, .. } => match subscribing {
                Subscribing::State { key, .. } => weight::string(key),
                Subscribing::Tone { item, .. } => weight::item_heap(item),
                Subscribing::Stats { .. } => Some(0),
            },
            Declared::Player(PlayerEvent::Failed { detail, .. }) => weight::string(detail),
            Declared::PlaceState { state: MediaState::Failed { detail }, .. } => weight::string(detail),
            Declared::Query { query, .. } => query.weight(),
            _ => Some(0),
        };
        weight::sum([Some(SLOT), own])
    }

    /// Folds `newer` into `self` without walking any payload. The payload that was replaced is returned so the caller frees it outside the lock.
    pub fn absorb(&mut self, newer: Declared) -> (Option<Declared>, Merge) {
        match (&mut *self, newer) {
            (Declared::VideoWindow { active, sources, .. }, Declared::VideoWindow { active: next, sources: replacement, owner }) => {
                *active = next;
                match replacement {
                    None => (None, Merge::Kept),
                    Some(replacement) => (sources.replace(replacement).map(|old| Declared::VideoWindow { owner, active: 0, sources: Some(old) }), Merge::Replaced),
                }
            }
            (Declared::ImageWindow { active, pages, .. }, Declared::ImageWindow { active: next, pages: replacement, owner }) => {
                *active = next;
                match replacement {
                    None => (None, Merge::Kept),
                    Some(replacement) => (pages.replace(replacement).map(|old| Declared::ImageWindow { owner, active: 0, pages: Some(old) }), Merge::Replaced),
                }
            }
            (Declared::Memory(held), Declared::Memory(next)) => {
                if *held != Pressure::Critical {
                    *held = next;
                }
                (None, Merge::Kept)
            }
            (slot, newer) => (Some(std::mem::replace(slot, newer)), Merge::Replaced),
        }
    }
}
