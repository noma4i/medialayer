use super::ingress::{Declared, PlaceInput, InputQueue, WindowKind};
use super::layer::WindowReadyListener;
use super::rows::RowLayout;
use crate::api::{MediaError, MediaItem, MediaState};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// One mounted place: immutable item, unique id, released at most once.
#[derive(uniffi::Object)]
pub struct MediaPlace {
    queue: Arc<InputQueue>,
    owner: u64,
}

impl MediaPlace {
    pub(super) fn new(queue: Arc<InputQueue>, owner: u64) -> MediaPlace {
        MediaPlace { queue, owner }
    }
}

#[uniffi::export]
impl MediaPlace {
    pub fn update(&self, input: PlaceInput) -> Result<(), MediaError> {
        self.queue.submit(Declared::PlaceUpdate { owner: self.owner, input })
    }

    pub fn seek(&self, fraction: f64) -> Result<(), MediaError> {
        self.queue.submit(Declared::PlaceSeek { owner: self.owner, fraction })
    }

    pub fn item_state(&self, state: MediaState) -> Result<(), MediaError> {
        self.queue.submit(Declared::PlaceState { owner: self.owner, state })
    }

    pub fn release(&self) {
        self.queue.release(self.owner);
    }
}

impl Drop for MediaPlace {
    fn drop(&mut self) {
        self.release();
    }
}

/// A window of sources with its own id: the DSL name picks a policy, never an identity.
#[derive(uniffi::Object)]
pub struct MediaWindow {
    queue: Arc<InputQueue>,
    owner: u64,
    kind: WindowKind,
    layout: Mutex<RowLayout>,
}

impl MediaWindow {
    pub(super) fn new(queue: Arc<InputQueue>, owner: u64, kind: WindowKind) -> MediaWindow {
        MediaWindow { queue, owner, kind, layout: Mutex::new(RowLayout::new(1, false)) }
    }

    pub(super) fn lay_out(&self, rows_per_page: u32, reversed: bool) {
        *self.layout() = RowLayout::new(rows_per_page, reversed);
    }

    fn layout(&self) -> MutexGuard<'_, RowLayout> {
        self.layout.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wrong_kind(&self) -> MediaError {
        MediaError::Unexpected { detail: "the window kind does not accept this call".to_string() }
    }
}

#[uniffi::export]
impl MediaWindow {
    pub fn replace(&self, active: u32, sources: Vec<Option<MediaItem>>) -> Result<(), MediaError> {
        if self.kind != WindowKind::Video {
            return Err(self.wrong_kind());
        }
        self.queue.submit(Declared::VideoWindow { owner: self.owner, active, sources: Some(Arc::new(sources)) })
    }

    pub fn replace_pages(&self, active: u32, pages: Vec<Vec<MediaItem>>) -> Result<(), MediaError> {
        if matches!(self.kind, WindowKind::Video) {
            return Err(self.wrong_kind());
        }
        self.layout().move_to(active);
        self.queue.submit(Declared::ImageWindow { owner: self.owner, active, pages: Some(Arc::new(pages)) })
    }

    /// Replaces the rows of an image window. `rows_per_page` rows form a page, a reversed list counts pages from its last row,
    /// and the active page stays where the last move or `show_rows` put it.
    pub fn replace_rows(&self, rows: Vec<Vec<MediaItem>>) -> Result<(), MediaError> {
        if matches!(self.kind, WindowKind::Video) {
            return Err(self.wrong_kind());
        }
        let (active, pages) = {
            let mut layout = self.layout();
            let pages = layout.paged(rows);
            (layout.active(), pages)
        };
        self.queue.submit(Declared::ImageWindow { owner: self.owner, active, pages: Some(Arc::new(pages)) })
    }

    /// Moves an image window to the page of the first visible row, the last one in a reversed list. `first` and `last` are the ends of the visible rows.
    pub fn show_rows(&self, first: u32, last: u32) -> Result<(), MediaError> {
        if matches!(self.kind, WindowKind::Video) {
            return Err(self.wrong_kind());
        }
        let active = {
            let mut layout = self.layout();
            layout.show(first, last);
            layout.active()
        };
        self.queue.submit(Declared::ImageWindow { owner: self.owner, active, pages: None })
    }

    /// Reports whether the active page is prepared: a video window by the first frame or the cache, an image window once every source of the page is cached or failed.
    pub fn subscribe_ready(&self, listener: Arc<dyn WindowReadyListener>) -> Result<(), MediaError> {
        self.queue.submit(Declared::WindowReady { owner: self.owner, listener })
    }

    pub fn move_to(&self, active: u32) -> Result<(), MediaError> {
        self.layout().move_to(active);
        match self.kind {
            WindowKind::Video => self.queue.submit(Declared::VideoWindow { owner: self.owner, active, sources: None }),
            WindowKind::Image(_) => self.queue.submit(Declared::ImageWindow { owner: self.owner, active, pages: None }),
        }
    }

    pub fn set_active(&self, is_active: bool) -> Result<(), MediaError> {
        self.queue.submit(Declared::WindowActive { owner: self.owner, is_active })
    }

    pub fn release(&self) {
        self.queue.release(self.owner);
    }
}

impl Drop for MediaWindow {
    fn drop(&mut self) {
        self.release();
    }
}

/// Sources the app wants stored whole: one load per source for all holders, cancelled when the last holder lets go.
#[derive(uniffi::Object)]
pub struct MediaWholeDeclaration {
    queue: Arc<InputQueue>,
    owner: u64,
}

impl MediaWholeDeclaration {
    pub(super) fn new(queue: Arc<InputQueue>, owner: u64) -> MediaWholeDeclaration {
        MediaWholeDeclaration { queue, owner }
    }
}

#[uniffi::export]
impl MediaWholeDeclaration {
    pub fn add(&self, items: Vec<MediaItem>) -> Result<(), MediaError> {
        self.queue.submit(Declared::WholeAdd { owner: self.owner, items })
    }

    pub fn remove(&self, items: Vec<MediaItem>) -> Result<(), MediaError> {
        self.queue.submit(Declared::WholeRemove { owner: self.owner, items })
    }

    pub fn release(&self) {
        self.queue.release(self.owner);
    }
}

impl Drop for MediaWholeDeclaration {
    fn drop(&mut self) {
        self.release();
    }
}

/// While at least one hold lives, a call runs: nothing plays, `play_url` is refused, and releasing restores nothing.
#[derive(uniffi::Object)]
pub struct CallHold {
    queue: Arc<InputQueue>,
    owner: u64,
}

impl CallHold {
    pub(super) fn new(queue: Arc<InputQueue>, owner: u64) -> CallHold {
        CallHold { queue, owner }
    }
}

#[uniffi::export]
impl CallHold {
    pub fn release(&self) {
        self.queue.release(self.owner);
    }
}

impl Drop for CallHold {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(uniffi::Object)]
pub struct PlaybackSuspension {
    queue: Arc<InputQueue>,
    owner: u64,
}

impl PlaybackSuspension {
    pub(super) fn new(queue: Arc<InputQueue>, owner: u64) -> PlaybackSuspension {
        PlaybackSuspension { queue, owner }
    }
}

#[uniffi::export]
impl PlaybackSuspension {
    pub fn release(&self) {
        self.queue.release(self.owner);
    }
}

impl Drop for PlaybackSuspension {
    fn drop(&mut self) {
        self.release();
    }
}
