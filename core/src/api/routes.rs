use super::error::MediaError;
use super::ports::CacheControl;
use super::source::{is_local_url, MediaItem, PlaybackRoute};
use super::types::Kind;
use crate::hls::is_manifest_url;
use crate::image_size::{parse, ImageSize};
use crate::proxy::{block_key, BLOCK_SIZE, Playhead, Priority, Proxy, Readiness};
use crate::session::RouteStaleness;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncReadExt;
use url::Url;

pub const SIZE_MEMO_ENTRIES: usize = 256;

#[derive(Default)]
struct SizeMemo {
    order: VecDeque<String>,
    sizes: HashMap<String, ImageSize>,
}

impl SizeMemo {
    fn insert(&mut self, key: String, size: ImageSize) {
        if self.sizes.insert(key.clone(), size).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > SIZE_MEMO_ENTRIES {
            if let Some(oldest) = self.order.pop_front() {
                self.sizes.remove(&oldest);
            }
        }
    }

    fn forget(&mut self, url: &str) {
        let matches = |key: &str| key == url || key.strip_prefix(url).is_some_and(|suffix| suffix.starts_with('#'));
        self.sizes.retain(|key, _| !matches(key));
        self.order.retain(|key| !matches(key));
    }
}

pub struct Routes {
    proxy: Proxy,
    cache: Arc<dyn CacheControl>,
    sizes: Mutex<SizeMemo>,
    #[cfg(test)]
    barrier: Barrier,
    #[cfg(test)]
    storage_lookups: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
pub struct Barrier {
    held: tokio::sync::watch::Sender<bool>,
    entered: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl Barrier {
    async fn pass(&self) {
        self.entered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        crate::test_signal::fire();
        let _ = self.held.subscribe().wait_for(|held| !*held).await;
    }
}

impl Routes {
    pub fn new(proxy: Proxy, cache: Arc<dyn CacheControl>) -> Routes {
        Routes {
            proxy,
            cache,
            sizes: Mutex::new(SizeMemo::default()),
            #[cfg(test)]
            barrier: Barrier { held: tokio::sync::watch::channel(false).0, entered: std::sync::atomic::AtomicUsize::new(0) },
            #[cfg(test)]
            storage_lookups: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    #[cfg(test)]
    pub fn hold_repair_and_purge(&self, held: bool) {
        self.barrier.held.send_replace(held);
    }

    #[cfg(test)]
    pub fn barrier_entries(&self) -> usize {
        self.barrier.entered.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(test)]
    pub fn storage_lookups(&self) -> usize {
        self.storage_lookups.load(std::sync::atomic::Ordering::SeqCst)
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
        let repaired = self.proxy.repair().await;
        #[cfg(test)]
        self.barrier.pass().await;
        repaired
    }

    pub fn readiness(&self) -> Readiness {
        self.proxy.readiness()
    }

    pub async fn image_bytes(&self, url: &str, headers: &HashMap<String, String>, limit: u64) -> Result<crate::proxy::ImageFetchResult, MediaError> {
        if is_local_url(url) {
            return read_local_file(url, limit).await.map(|bytes| crate::proxy::ImageFetchResult { bytes, source: crate::proxy::ImageOrigin::Local });
        }
        self.proxy.fetch_bytes(url, headers, limit).await.map_err(|error| MediaError::Fetch { detail: error.to_string() })
    }

    pub async fn reserve_decode(&self, bytes: u64) -> Result<crate::memory::Reservation, MediaError> {
        self.proxy.reserve_decode(bytes).await.map_err(|error| MediaError::Overloaded { detail: error.to_string() })
    }

    pub async fn reserve_capture(&self, bytes: u64) -> Result<crate::memory::Reservation, MediaError> {
        self.proxy.reserve_capture(bytes).await.map_err(|error| MediaError::Overloaded { detail: error.to_string() })
    }

    pub async fn image_size(&self, url: &str, headers: &HashMap<String, String>) -> Result<Option<ImageSize>, MediaError> {
        let local = is_local_url(url);
        let key = if local { url.to_string() } else { self.proxy.key_for(url, headers) };
        if let Some(size) = self.known_size(&key) {
            return Ok(Some(size));
        }
        let (stored, head) = if local {
            (key.clone(), read_local_head(url, BLOCK_SIZE).await?)
        } else {
            self.network_head(url, headers).await?
        };
        let size = parse(&head);
        if let Some(size) = size {
            if local || !self.cache.set_image_size(&stored, size) {
                locked(&self.sizes).insert(key, size);
            }
        }
        Ok(size)
    }

    fn known_size(&self, key: &str) -> Option<ImageSize> {
        locked(&self.sizes).sizes.get(key).copied().or_else(|| self.cache.image_size(key)).or_else(|| self.cache.image_size(&block_key(key, 0)))
    }

    async fn network_head(&self, url: &str, headers: &HashMap<String, String>) -> Result<(String, Vec<u8>), MediaError> {
        if let Some(head) = self.proxy.read_head(url, headers, BLOCK_SIZE).await {
            return Ok(head);
        }
        let failed = |detail: String| MediaError::Fetch { detail };
        self.proxy.fetch_first_block(url, headers, Priority::Image).await.map_err(|error| failed(error.to_string()))?;
        self.proxy.read_head(url, headers, BLOCK_SIZE).await.ok_or_else(|| failed("the response vanished from the cache".to_string()))
    }

    pub async fn purge(&self, url: &str) {
        locked(&self.sizes).forget(url);
        self.cache.purge(url).await;
        #[cfg(test)]
        self.barrier.pass().await;
    }

    pub async fn is_cached(&self, url: &str, headers: &HashMap<String, String>) -> bool {
        if is_local_url(url) {
            return true;
        }
        #[cfg(test)]
        self.storage_lookups.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let key = self.proxy.key_for(url, headers);
        self.cache.contains(&key).await || self.cache.contains(&block_key(&key, 0)).await
    }
}

fn locked<V>(mutex: &Mutex<V>) -> std::sync::MutexGuard<'_, V> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

async fn open_local(url: &str) -> Result<tokio::fs::File, MediaError> {
    let failed = |detail: String| MediaError::Fetch { detail };
    let path = Url::parse(url).ok().and_then(|parsed| parsed.to_file_path().ok()).ok_or_else(|| failed(format!("{url} is not a file path")))?;
    tokio::fs::File::open(&path).await.map_err(|error| failed(error.to_string()))
}

async fn read_local_head(url: &str, length: u64) -> Result<Vec<u8>, MediaError> {
    let mut bytes = Vec::new();
    open_local(url).await?.take(length).read_to_end(&mut bytes).await.map_err(|error| MediaError::Fetch { detail: error.to_string() })?;
    Ok(bytes)
}

async fn read_local_file(url: &str, limit: u64) -> Result<Vec<u8>, MediaError> {
    let failed = |detail: String| MediaError::Fetch { detail };
    let file = open_local(url).await?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes).await.map_err(|error| failed(error.to_string()))?;
    if bytes.len() as u64 > limit {
        return Err(failed(format!("file of more than {limit} bytes exceeds the limit")));
    }
    Ok(bytes)
}
