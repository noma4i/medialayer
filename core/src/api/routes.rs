use super::error::MediaError;
use super::ports::CacheControl;
use super::source::{MediaItem, PlaybackRoute};
use super::types::Kind;
use crate::hls::is_manifest_url;
use crate::proxy::{block_key, Proxy, Readiness};
use crate::session::RouteStaleness;
use std::collections::HashMap;
use std::sync::Arc;

pub struct Routes {
    proxy: Proxy,
    cache: Arc<dyn CacheControl>,
}

impl Routes {
    pub fn new(proxy: Proxy, cache: Arc<dyn CacheControl>) -> Routes {
        Routes { proxy, cache }
    }

    pub fn play_url(&self, source: &MediaItem) -> PlaybackRoute {
        let (hls, looping) = (is_manifest_url(&source.url), source.kind == Kind::Video);
        match self.proxy.url_for(&source.url, &source.headers) {
            Some(url) => PlaybackRoute { url, headers: HashMap::new(), generation: Some(self.proxy.route_generation()), hls, looping },
            None => PlaybackRoute { url: source.url.clone(), headers: source.headers.clone(), generation: None, hls, looping },
        }
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

    pub fn readiness(&self) -> Readiness {
        self.proxy.readiness()
    }

    pub async fn image_bytes(&self, url: &str, headers: &HashMap<String, String>, limit: u64) -> Result<Vec<u8>, MediaError> {
        self.proxy.fetch_bytes(url, headers, limit).await.map_err(|error| MediaError::Fetch { detail: error.to_string() })
    }

    pub async fn purge(&self, url: &str) {
        self.cache.purge(url).await;
    }

    pub async fn is_cached(&self, url: &str, headers: &HashMap<String, String>) -> bool {
        let key = self.proxy.key_for(url, headers);
        self.cache.contains(&key).await || self.cache.contains(&block_key(&key, 0)).await
    }
}
