use super::types::Kind;
use crate::prefetch::{Source, SourceKind};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MediaItem {
    pub kind: Kind,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub cover: Option<String>,
    pub blur_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlaybackRoute {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub generation: Option<u64>,
}

impl MediaItem {
    pub fn bare(url: &str, kind: Kind) -> MediaItem {
        MediaItem { kind, url: url.to_string(), headers: HashMap::new(), cover: None, blur_hash: None }
    }

    #[cfg(test)]
    pub fn bare_video(url: &str) -> MediaItem {
        MediaItem::bare(url, Kind::Video)
    }

    pub fn for_prefetch(&self) -> Source {
        let kind = match self.kind {
            Kind::Image => SourceKind::Image,
            Kind::Video => SourceKind::Video,
            Kind::Audio => SourceKind::Audio,
        };
        Source { url: self.url.clone(), headers: self.headers.clone(), kind }
    }
}
