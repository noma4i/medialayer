use super::error::MediaError;
use super::ports::CacheControl;
use super::source::{is_local_url, MediaItem, PlaybackRoute};
use super::types::Kind;
use crate::hls::is_manifest_url;
use crate::proxy::{block_key, Playhead, Proxy, Readiness};
use crate::session::RouteStaleness;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use url::Url;

pub struct Routes {
    proxy: Proxy,
    cache: Arc<dyn CacheControl>,
}

impl Routes {
    pub fn new(proxy: Proxy, cache: Arc<dyn CacheControl>) -> Routes {
        Routes { proxy, cache }
    }

    pub fn play_url(&self, source: &MediaItem) -> PlaybackRoute {
        let (hls, looping) = (is_manifest_url(&source.url), source.kind == Kind::Video && source.looping);
        match self.proxy.url_for(&source.url, &source.headers) {
            Some(url) => PlaybackRoute { url, headers: HashMap::new(), generation: Some(self.proxy.route_generation()), hls, looping },
            None => PlaybackRoute { url: source.url.clone(), headers: source.headers.clone(), generation: None, hls, looping },
        }
    }

    pub fn set_playhead(&self, playhead: Arc<dyn Playhead>) {
        self.proxy.set_playhead(playhead);
    }

    pub fn set_readahead_percent(&self, percent: u32) {
        self.proxy.set_readahead_percent(percent);
    }

    pub fn nudge_window(&self, playlist: &str) {
        self.proxy.nudge_window(playlist);
    }

    pub fn window_hint(&self, playlist: &str) -> Option<u64> {
        self.proxy.window_hint(playlist)
    }

    pub fn grow_window(&self, playlist: &str) {
        self.proxy.grow_window(playlist);
    }

    pub fn is_stale(&self, route: &PlaybackRoute) -> bool {
        RouteStaleness::is_stale(true, route.generation, self.generation(), self.readiness())
    }

    pub fn refresh(&self, source: &MediaItem, route: &PlaybackRoute) -> PlaybackRoute {
        if self.is_stale(route) {
            return self.play_url(source);
        }
        route.clone()
    }

    pub fn generation(&self) -> u64 {
        self.proxy.route_generation()
    }

    pub async fn repair(&self) -> bool {
        self.proxy.repair().await
    }

    pub fn readiness(&self) -> Readiness {
        self.proxy.readiness()
    }

    pub async fn image_bytes(&self, url: &str, headers: &HashMap<String, String>, limit: u64) -> Result<Vec<u8>, MediaError> {
        if is_local_url(url) {
            return read_local_file(url, limit).await;
        }
        self.proxy.fetch_bytes(url, headers, limit).await.map_err(|error| MediaError::Fetch { detail: error.to_string() })
    }

    pub async fn purge(&self, url: &str) {
        self.cache.purge(url).await;
    }

    pub async fn is_cached(&self, url: &str, headers: &HashMap<String, String>) -> bool {
        if is_local_url(url) {
            return true;
        }
        let key = self.proxy.key_for(url, headers);
        self.cache.contains(&key).await || self.cache.contains(&block_key(&key, 0)).await
    }
}

async fn read_local_file(url: &str, limit: u64) -> Result<Vec<u8>, MediaError> {
    let failed = |detail: String| MediaError::Fetch { detail };
    let path = Url::parse(url).ok().and_then(|parsed| parsed.to_file_path().ok()).ok_or_else(|| failed(format!("{url} is not a file path")))?;
    let file = tokio::fs::File::open(&path).await.map_err(|error| failed(error.to_string()))?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes).await.map_err(|error| failed(error.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(failed(format!("file of more than {limit} bytes exceeds the limit")));
    }
    Ok(bytes)
}
