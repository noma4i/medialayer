use super::types::Kind;
use crate::prefetch::{CacheStrategy, Source, SourceKind};
use std::collections::HashMap;
use url::Url;

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

pub const BLANK_URL_DETAIL: &str = "the source has no url";

const LOCAL_SCHEMES: [&str; 3] = ["file://", "android.resource://", "asset:///"];

pub fn is_local_url(url: &str) -> bool {
    LOCAL_SCHEMES.iter().any(|scheme| url.starts_with(scheme))
}

fn trimmed(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// The one cleaning rule of a source value: blank text is absent, the url loses its edge whitespace.
#[uniffi::export]
pub fn normalize_item(item: MediaItem) -> MediaItem {
    let (cover, blur_hash) = (item.cover.as_deref().and_then(trimmed), item.blur_hash.as_deref().and_then(trimmed));
    MediaItem { url: item.url.trim().to_string(), cover, blur_hash, ..item }
}

/// An absolute path or a `file://` URL as a canonical `file://` URL, or an empty string when the text names no file.
#[uniffi::export]
pub fn local_file_url(path: String) -> String {
    let text = path.trim();
    let parsed = if text.starts_with("file://") { Url::parse(text).ok().filter(|url| url.to_file_path().is_ok()) } else { Url::from_file_path(text).ok() };
    parsed.map_or_else(String::new, |url| url.to_string())
}

/// An `http` or `https` URL without its edge whitespace, or an empty string when the text is not one.
#[uniffi::export]
pub fn remote_url(text: String) -> String {
    let text = text.trim();
    let remote = MediaItem::bare(text, Kind::Video).is_remote() && Url::parse(text).is_ok_and(|url| url.host_str().is_some());
    if remote { text.to_string() } else { String::new() }
}

/// The refusal of a bundled source of this kind, the same text on both platforms.
#[uniffi::export]
pub fn bundled_source_error(kind: Kind) -> Option<String> {
    (kind == Kind::Image).then(|| "a bundled source is a video or an audio, not an image".to_string())
}

impl MediaItem {
    pub fn bare(url: &str, kind: Kind) -> MediaItem {
        MediaItem { kind, url: url.to_string(), headers: HashMap::new(), cover: None, blur_hash: None, looping: true, strategy: CacheStrategy::Window }
    }

    pub fn has_url(&self) -> bool {
        !self.url.trim().is_empty()
    }

    pub fn is_local(&self) -> bool {
        is_local_url(&self.url)
    }

    pub fn is_remote(&self) -> bool {
        self.url.split_once("://").is_some_and(|(scheme, rest)| !rest.is_empty() && (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")))
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
