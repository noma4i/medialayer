use super::ingress::{Declared, PlaceInput, InputQueue, WindowKind};
use crate::api::{MediaError, MediaItem, MediaState};
use std::sync::Arc;

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
}

impl MediaWindow {
    pub(super) fn new(queue: Arc<InputQueue>, owner: u64, kind: WindowKind) -> MediaWindow {
        MediaWindow { queue, owner, kind }
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
        self.queue.submit(Declared::ImageWindow { owner: self.owner, active, pages: Some(Arc::new(pages)) })
    }

    pub fn move_to(&self, active: u32) -> Result<(), MediaError> {
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
