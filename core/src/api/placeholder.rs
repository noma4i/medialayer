use super::source::MediaItem;
use crate::memory::{bounded, InputLimits};
use super::types::Kind;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Placeholder {
    BlurHash { hash: String },
    Cover { url: String },
    FirstFrame,
    None,
}

#[uniffi::export]
pub fn placeholder_for(source: MediaItem, ready: bool) -> Placeholder {
    match source.kind {
        Kind::Image if ready => Placeholder::None,
        Kind::Image => source
            .blur_hash
            .filter(|hash| bounded(hash.len() as u64, InputLimits::default().blurhash_chars).is_ok())
            .map_or(Placeholder::None, |hash| Placeholder::BlurHash { hash }),
        Kind::Video if ready => Placeholder::None,
        Kind::Video if source.is_local() => match (&source.cover, source.url.starts_with("file://")) {
            (None, true) => Placeholder::FirstFrame,
            _ => Placeholder::None,
        },
        Kind::Video => source.cover.map_or(Placeholder::FirstFrame, |url| Placeholder::Cover { url }),
        Kind::Audio => Placeholder::None,
    }
}
