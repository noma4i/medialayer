use super::types::Kind;
use crate::prefetch::{CacheStrategy, Source, SourceKind};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MediaItem {
    pub kind: Kind,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub cover: Option<String>,
    pub blur_hash: Option<String>,
    #[uniffi(default = true)]
    pub looping: bool,
    pub strategy: CacheStrategy,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlaybackRoute {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub generation: Option<u64>,
    pub hls: bool,
    pub looping: bool,
}

pub fn is_local_url(url: &str) -> bool {
    url.starts_with("file://")
}

impl MediaItem {
    pub fn bare(url: &str, kind: Kind) -> MediaItem {
        MediaItem { kind, url: url.to_string(), headers: HashMap::new(), cover: None, blur_hash: None, looping: true, strategy: CacheStrategy::Window }
    }

    pub fn is_local(&self) -> bool {
        is_local_url(&self.url)
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
        Source { url: self.url.clone(), headers: self.headers.clone(), kind, strategy: self.strategy }
    }
}
