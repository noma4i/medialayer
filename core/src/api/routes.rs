use super::ports::CacheControl;
use super::source::{MediaItem, PlaybackRoute};
use crate::proxy::{Proxy, Readiness};
use crate::session::{Readiness as SessionReadiness, RouteStaleness};
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
        match self.proxy.url_for(&source.url, &source.headers) {
            Some(url) => PlaybackRoute { url, headers: HashMap::new(), generation: Some(self.proxy.route_generation()) },
            None => PlaybackRoute { url: source.url.clone(), headers: source.headers.clone(), generation: None },
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

    pub fn readiness(&self) -> SessionReadiness {
        match self.proxy.readiness() {
            Readiness::Stopped => SessionReadiness::Stopped,
            Readiness::Ready => SessionReadiness::Ready,
            Readiness::Unhealthy => SessionReadiness::Unhealthy,
        }
    }

    pub async fn purge(&self, url: &str) {
        self.cache.purge(url).await;
    }

    pub async fn is_cached(&self, url: &str) -> bool {
        self.cache.contains(url).await
    }
}
