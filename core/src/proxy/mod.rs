mod buffer;
mod byte_store;
mod config;
mod flight;
mod inner;
mod route;
mod serve;
mod serve_blocks;
mod serve_origin;
mod serve_reply;
mod server;
mod slots;
mod tasks;
mod cursor;
mod window;
mod playlists;
mod recent;
pub use playlists::StoredPlaylist;

#[cfg(test)]
pub(crate) mod memory_store;
#[cfg(test)]
pub(crate) mod test_client;

pub use byte_store::{ByteStore, Meta, StoreReader, StoreWriter};
pub use config::{Config, BLOCK_SIZE};
pub use route::block_key;
pub use slots::Priority;
pub(crate) use slots::closing_inline;
pub use window::{Playhead, PositionProbe};

use crate::fetch::Fetcher;
use crate::limits::Limits;
use flight::FlightOutcome;
use inner::ProxyShared;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Notify;

const FETCH_ATTEMPTS: usize = 3;
const PING_REQUEST: &str = "GET /ping HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
const PING_REPLY_LIMIT: u64 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ImageOrigin {
    Cache,
    Network,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ImageFetchResult {
    pub bytes: Vec<u8>,
    pub source: ImageOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Stopped,
    Ready,
    Unhealthy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairOutcome {
    Healthy,
    Rebound,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProxyError {
    #[error("bind failed: {0}")]
    Bind(String),
    #[error("origin fetch failed: {0}")]
    Fetch(String),
    #[error("origin answered status {0}")]
    Status(u16),
    #[error("store refused the write")]
    StoreRefused,
    #[error("prefetch queue is full")]
    QueueFull,
}

#[derive(Clone)]
pub struct Proxy {
    inner: Arc<ProxyShared>,
    _owner: Arc<tasks::TasksOwner>,
}

impl Proxy {
    #[cfg(test)]
    fn hold_rebind(&self, hold: bool) {
        if hold { self.inner.rebind_entered.store(false, Ordering::SeqCst); }
        self.inner.rebind_hold.send_replace(hold);
    }
    #[cfg(test)]
    fn rebind_entered(&self) -> bool { self.inner.rebind_entered.load(Ordering::SeqCst) }
    #[cfg(test)]
    fn fail_accepts(&self) { self.inner.accept_failures.send_replace(true); }
    #[cfg(test)]
    pub(crate) fn playhead_holds(&self) -> usize {
        self.inner.windows.playhead_holds()
    }

    #[cfg(test)]
    pub(crate) fn gate_hold(&self, target: &str) -> (Arc<std::sync::atomic::AtomicBool>, Arc<tokio::sync::Semaphore>) {
        self.inner.windows.gate_hold(target)
    }

    #[cfg(test)]
    pub(crate) fn queued_requests(&self) -> usize {
        self.inner.slots.waiting() + self.inner.image_slots.waiting()
    }
    pub async fn start(config: Config, fetcher: Arc<dyn Fetcher>, store: Arc<dyn ByteStore>) -> Result<Proxy, ProxyError> {
        let tasks = tasks::Tasks::default();
        let inner = Arc::new(ProxyShared::new(config, fetcher, store, tasks.clone()));
        let proxy = Proxy { inner, _owner: Arc::new(tasks::TasksOwner(tasks)) };
        proxy.restart(0).await?;
        Ok(proxy)
    }

    pub fn failures(&self) -> Arc<crate::failures::Failures> {
        self.inner.config.failures.clone()
    }

    pub fn port(&self) -> u16 {
        self.inner.port.load(Ordering::SeqCst)
    }

    pub fn route_generation(&self) -> u64 {
        self.inner.route_generation.load(Ordering::SeqCst)
    }

    pub fn live_tasks(&self) -> usize {
        self.inner.live.load(Ordering::SeqCst)
    }

    pub fn readiness(&self) -> Readiness {
        if !self.inner.running.load(Ordering::SeqCst) {
            Readiness::Stopped
        } else if self.inner.alive.load(Ordering::SeqCst) != 0 && self.port() > 0 {
            Readiness::Ready
        } else {
            Readiness::Unhealthy
        }
    }

    pub fn url_for(&self, original: &str, headers: &HashMap<String, String>) -> Option<String> {
        if self.readiness() != Readiness::Ready || !(original.starts_with("http://") || original.starts_with("https://")) {
            return None;
        }
        self.inner.register_headers(original, headers);
        Some(route::proxy_url(self.port(), original))
    }

    pub fn set_limits(&self, limits: &Limits) {
        self.forget_playlists(None);
        self.inner.set_timeout(Duration::from_secs(limits.timeout_seconds.into()));
        self.inner.slots.set_max(limits.max_concurrent as usize);
        self.inner.image_slots.set_max(limits.max_concurrent as usize);
        self.inner.windows.set_chunks_ahead(limits.playback_chunks_ahead);
    }

    pub fn set_playhead(&self, playhead: Arc<dyn Playhead>) {
        self.inner.windows.set_playhead(playhead);
    }

    pub fn set_readahead_percent(&self, percent: u32) {
        self.inner.windows.set_scale_percent(percent);
    }

    pub fn nudge_window(&self, playlist: &str) {
        self.inner.windows.nudge(playlist);
    }

    pub fn window_hint(&self, playlist: &str) -> Option<u64> {
        self.inner.windows.hint_for(playlist)
    }

    pub fn grow_window(&self, playlist: &str) {
        self.inner.windows.grow(playlist);
    }

    pub fn key_for(&self, url: &str, headers: &HashMap<String, String>) -> String {
        self.inner.key(url, &self.inner.upstream_headers(url, headers))
    }

    pub async fn fetch_bytes(&self, url: &str, headers: &HashMap<String, String>, limit: u64) -> Result<ImageFetchResult, ProxyError> {
        let source = match self.fetch_into_store(url, headers, Priority::Image, None).await {
            Ok(source) => source,
            Err(ProxyError::StoreRefused) => return self.fetch_direct(url, headers, limit).await,
            Err(error) => return Err(error),
        };
        let key = self.key_for(url, headers);
        match self.inner.store.len(&key).await {
            Some(length) if length > limit => {
                self.inner.store.remove(&key).await;
                Err(ProxyError::Fetch(format!("response of {length} bytes exceeds the limit of {limit}")))
            }
            _ => match self.read_stored(url, headers, limit).await {
                Some(bytes) => Ok(ImageFetchResult { bytes, source }),
                None => {
                    self.inner.config.failures.report(crate::failures::MediaFailure {
                        kind: crate::api::Kind::Image,
                        url: url.to_string(),
                        stage: crate::failures::FailureStage::Store,
                        reason: "the stored response could not be read".to_string(),
                    });
                    self.fetch_direct(url, headers, limit).await
                }
            },
        }
    }

    async fn fetch_direct(&self, url: &str, headers: &HashMap<String, String>, limit: u64) -> Result<ImageFetchResult, ProxyError> {
        let upstream = self.inner.upstream_headers(url, headers);
        let Ok(permit) = self.inner.gate(Priority::Image).acquire(Priority::Image).await else {
            return Err(ProxyError::QueueFull);
        };
        let timeout = self.inner.timeout();
        let request = crate::fetch::Request { url: url.to_string(), headers: upstream, range: None };
        let outcome = match tokio::time::timeout(timeout, self.inner.fetcher.fetch(request)).await {
            Err(_) => Err(ProxyError::Fetch("origin timed out".to_string())),
            Ok(Err(crate::fetch::FetchError::Status(status))) => Err(ProxyError::Status(status)),
            Ok(Err(error)) => Err(ProxyError::Fetch(error.to_string())),
            Ok(Ok(response)) if response.status != 200 => Err(ProxyError::Status(response.status)),
            Ok(Ok(response)) => {
                let body = permit.hold(response.body);
                match flight::collect_limited(body, limit as usize, timeout, &self.inner.buffers()).await {
                    Some(bytes) => Ok(ImageFetchResult { bytes, source: ImageOrigin::Network }),
                    None => Err(ProxyError::Fetch(format!("response is incomplete or exceeds the limit of {limit}"))),
                }
            }
        };
        if let Err(error) = &outcome {
            self.inner.config.failures.report(crate::failures::MediaFailure {
                kind: crate::api::Kind::Image,
                url: url.to_string(),
                stage: crate::failures::FailureStage::Load,
                reason: error.to_string(),
            });
        }
        outcome
    }

    pub async fn reserve_decode(&self, bytes: u64) -> Result<crate::memory::Reservation, ProxyError> {
        let bytes = usize::try_from(bytes).map_err(|_| ProxyError::Fetch("the decode does not fit the address space".to_string()))?;
        self.inner.buffers().reserve_decode(bytes).await.map_err(|refused| ProxyError::Fetch(refused.to_string()))
    }

    pub async fn reserve_capture(&self, bytes: u64) -> Result<crate::memory::Reservation, ProxyError> {
        let bytes = usize::try_from(bytes).map_err(|_| ProxyError::Fetch("the capture does not fit the address space".to_string()))?;
        self.inner.buffers().reserve_capture(bytes).await.map_err(|refused| ProxyError::Fetch(refused.to_string()))
    }

    pub async fn read_stored(&self, url: &str, headers: &HashMap<String, String>, limit: u64) -> Option<Vec<u8>> {
        let key = self.key_for(url, headers);
        let length = self.inner.store.len(&key).await.filter(|length| *length <= limit)?;
        let _held = self.inner.buffers().reserve_exact(length as usize).await.ok()?;
        self.inner.store.get_range(&key, 0, length).await
    }

    pub async fn read_head(&self, url: &str, headers: &HashMap<String, String>, limit: u64) -> Option<(String, Vec<u8>)> {
        let key = self.key_for(url, headers);
        for stored in [key.clone(), route::block_key(&key, 0)] {
            let Some(length) = self.inner.store.len(&stored).await else { continue };
            let length = length.min(limit);
            let _held = self.inner.buffers().reserve_exact(length as usize).await.ok()?;
            if let Some(bytes) = self.inner.store.get_range(&stored, 0, length).await {
                return Some((stored, bytes));
            }
        }
        None
    }

    pub async fn playlist_plan(&self, url: &str, headers: &HashMap<String, String>) -> Option<Arc<StoredPlaylist>> {
        self.inner.stored_plan(url, &self.key_for(url, headers)).await.ok().flatten()
    }

    pub fn forget_playlists(&self, url: Option<&str>) {
        self.inner.playlists.forget(url);
    }

    pub fn forget_downloads(&self, url: Option<&str>) {
        self.inner.playlists.forget(url);
        self.inner.forget_parked(url);
    }

    pub async fn ping(&self) -> bool {
        let port = self.port();
        if port == 0 {
            return false;
        }
        let exchange = async {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
            stream.write_all(PING_REQUEST.as_bytes()).await.ok()?;
            let mut reply = String::new();
            stream.take(PING_REPLY_LIMIT).read_to_string(&mut reply).await.ok()?;
            Some(reply.starts_with("HTTP/1.1 200") && reply.ends_with("pong"))
        };
        tokio::time::timeout(self.inner.config.ping_timeout, exchange).await.ok().flatten().unwrap_or(false)
    }

    pub async fn repair(&self) -> bool {
        self.repair_outcome().await != RepairOutcome::Failed
    }

    pub async fn repair_outcome(&self) -> RepairOutcome {
        let proxy = self.clone();
        tokio::spawn(async move { proxy.repair_inner().await }).await.unwrap_or(RepairOutcome::Failed)
    }

    async fn repair_inner(&self) -> RepairOutcome {
        let mut listener = self.inner.listener.lock().await;
        if self.inner.stopped.load(Ordering::SeqCst) {
            return RepairOutcome::Failed;
        }
        if self.ping().await {
            return RepairOutcome::Healthy;
        }
        let previous = self.inner.last_port.load(Ordering::SeqCst);
        match rebind(&self.inner, &mut listener, previous, "repair").await {
            Ok(_) => {},
            Err(error) => {
                crate::trace!("proxy", "repair failed: {error}");
                return RepairOutcome::Failed;
            }
        }
        RepairOutcome::Rebound
    }

    pub async fn stop(&self) {
        let mut listener = self.inner.listener.lock().await;
        self.inner.stopped.store(true, Ordering::SeqCst);
        self.shut_down(&mut listener).await;
        self.forget_playlists(None);
    }

    pub async fn restart(&self, preferred_port: u16) -> Result<u16, ProxyError> {
        let proxy = self.clone();
        tokio::spawn(async move {
            let mut listener = proxy.inner.listener.lock().await;
            proxy.inner.stopped.store(false, Ordering::SeqCst);
            rebind(&proxy.inner, &mut listener, preferred_port, "repair").await
        }).await.map_err(|error| ProxyError::Bind(error.to_string()))?
    }

    pub async fn stored_len(&self, url: &str, headers: &HashMap<String, String>) -> Option<u64> {
        let upstream = self.inner.upstream_headers(url, headers);
        self.inner.store.len(&self.inner.key(url, &upstream)).await
    }

    pub async fn remove_stored(&self, url: &str, headers: &HashMap<String, String>) {
        let upstream = self.inner.upstream_headers(url, headers);
        let key = self.inner.key(url, &upstream);
        self.inner.store.remove(&key).await;
        self.inner.forget_playlist(&key);
    }

    pub async fn fetch_into_store(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        priority: Priority,
        progress: Option<&Notify>,
    ) -> Result<ImageOrigin, ProxyError> {
        let upstream = self.inner.upstream_headers(url, headers);
        let key = self.inner.key(url, &upstream);
        if crate::hls::is_manifest_url(url) && self.inner.store.contains(&key).await {
            let plan = self.inner.stored_plan(url, &key).await.map_err(|message| ProxyError::Fetch(message.into()))?;
            if plan.is_none() {
                self.inner.store.remove(&key).await;
                self.inner.forget_playlist(&key);
            }
        }
        self.download_into_store(&key, progress, || self.inner.get_or_start(url, upstream.clone(), priority)).await
    }

    pub async fn fetch_block_into_store(
        &self,
        url: &str,
        block_index: u64,
        headers: &HashMap<String, String>,
        priority: Priority,
    ) -> Result<(), ProxyError> {
        let upstream = self.inner.upstream_headers(url, headers);
        let base = self.inner.key(url, &upstream);
        let key = route::block_key(&base, block_index);
        self.download_into_store(&key, None, || self.inner.get_or_start_block(url, block_index, None, upstream.clone(), priority))
            .await.map(|_| ())
    }

    pub async fn keep(&self, url: &str, headers: &HashMap<String, String>) {
        let upstream = self.inner.upstream_headers(url, headers);
        self.inner.store.keep(&self.inner.key(url, &upstream)).await;
    }

    pub async fn fetch_all_blocks(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        priority: Priority,
        progress: &Notify,
    ) -> Result<u64, ProxyError> {
        let upstream = self.inner.upstream_headers(url, headers);
        let base = self.inner.key(url, &upstream);
        let first = route::block_key(&base, 0);
        let started = self.download_into_store(&first, Some(progress), || self.inner.get_or_start_block(url, 0, None, upstream.clone(), priority)).await;
        if let Err(error) = started {
            if !self.inner.ignores_range(&base) {
                return Err(error);
            }
            return self.fetch_single_file(url, headers, priority, progress).await;
        }
        let Some(total) = self.inner.store.meta(&first).await.and_then(|meta| meta.total_length) else {
            return self.fetch_single_file(url, headers, priority, progress).await;
        };
        self.inner.store.keep(&first).await;
        for block in 1..total.div_ceil(self.inner.config.chunk_size) {
            let key = route::block_key(&base, block);
            self.download_into_store(&key, Some(progress), || self.inner.get_or_start_block(url, block, Some(total), upstream.clone(), priority)).await?;
            self.inner.store.keep(&key).await;
        }
        for block in 0..total.div_ceil(self.inner.config.chunk_size) {
            let key = route::block_key(&base, block);
            let expected = self.inner.config.chunk_size.min(total - block * self.inner.config.chunk_size);
            if self.inner.store.len(&key).await != Some(expected) {
                return Err(ProxyError::StoreRefused);
            }
        }
        Ok(total)
    }

    async fn fetch_single_file(&self, url: &str, headers: &HashMap<String, String>, priority: Priority, progress: &Notify) -> Result<u64, ProxyError> {
        self.fetch_into_store(url, headers, priority, Some(progress)).await?;
        Ok(self.stored_len(url, headers).await.unwrap_or(0))
    }

    pub async fn fetch_first_block(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        priority: Priority,
    ) -> Result<(), ProxyError> {
        self.fetch_block_into_store(url, 0, headers, priority).await
    }

    async fn download_into_store(
        &self,
        key: &str,
        progress: Option<&Notify>,
        start: impl Fn() -> flight::FlightSubscription,
    ) -> Result<ImageOrigin, ProxyError> {
        for _ in 0..FETCH_ATTEMPTS {
            if self.inner.store.contains(key).await {
                if let Some(progress) = progress { progress.notify_one(); }
                return Ok(ImageOrigin::Cache);
            }
            match start().wait_finished(progress).await.outcome {
                FlightOutcome::Complete | FlightOutcome::StoredWhole => {
                    if let Some(progress) = progress { progress.notify_one(); }
                    return Ok(ImageOrigin::Network);
                }
                FlightOutcome::Rejected(rejection) => return Err(ProxyError::Status(rejection.status)),
                FlightOutcome::Failed(message) => return Err(ProxyError::Fetch(message)),
                FlightOutcome::Unstorable => return Err(ProxyError::StoreRefused),
                FlightOutcome::QueueFull => return Err(ProxyError::QueueFull),
                FlightOutcome::Cancelled | FlightOutcome::Running => {}
            }
        }
        Err(ProxyError::Fetch("download cancelled".to_string()))
    }

    async fn shut_down(&self, listener: &mut Option<inner::Listener>) {
        self.inner.running.store(false, Ordering::SeqCst);
        self.inner.port.store(0, Ordering::SeqCst);
        let Some(stopped) = listener.take() else { return };
        let _ = stopped.shutdown.send(true);
        let _ = stopped.finished.await;
        self.inner.cancel_all_flights();
    }
}

fn rebind_after_accept_errors(inner: Arc<ProxyShared>, epoch: u64) {
    tokio::spawn(async move {
        let mut listener = inner.listener.lock().await;
        if inner.stopped.load(Ordering::SeqCst) || inner.listener_epoch.load(Ordering::SeqCst) != epoch { return; }
        let previous = inner.last_port.load(Ordering::SeqCst);
        if let Err(error) = rebind(&inner, &mut listener, previous, "accept-errors").await {
            crate::trace!("proxy", "repair failed: {error}");
        }
    });
}

async fn rebind(inner: &Arc<ProxyShared>, listener: &mut Option<inner::Listener>, preferred_port: u16, reason: &str) -> Result<u16, ProxyError> {
    #[cfg(test)]
    {
        inner.rebind_entered.store(true, Ordering::SeqCst);
        crate::test_signal::fire();
        let _ = inner.rebind_hold.subscribe().wait_for(|held| !*held).await;
    }
    let (started, port) = server::spawn_listener(inner, preferred_port).await?;
    let old = listener.replace(started);
    let previous = inner.last_port.swap(port, Ordering::SeqCst);
    if previous != 0 && previous != port { inner.route_generation.fetch_add(1, Ordering::SeqCst); }
    inner.port.store(port, Ordering::SeqCst);
    inner.running.store(true, Ordering::SeqCst);
    inner.playlists.forget(None);
    crate::trace!("heal", "proxy rebind reason={reason} port={previous}->{port}");
    #[cfg(test)]
    {
        inner.rebind_entered.store(false, Ordering::SeqCst);
        crate::test_signal::fire();
    }
    if let Some(old) = old {
        let _ = old.shutdown.send(true);
        let _ = old.finished.await;
    }
    Ok(port)
}

#[cfg(test)]
mod bounds_tests;
#[cfg(test)]
mod buffer_tests;
#[cfg(test)]
mod hold_tests;
#[cfg(test)]
mod lanes_tests;
#[cfg(test)]
mod limits_tests;
#[cfg(test)]
mod namespace_tests;
#[cfg(test)]
mod proxy_harness;
#[cfg(test)]
mod proxy_blocks_tests;
#[cfg(test)]
mod proxy_ignored_range_tests;
#[cfg(test)]
mod proxy_lifecycle_tests;
#[cfg(test)]
mod proxy_playlist_tests;
#[cfg(test)]
mod proxy_store_tests;
#[cfg(test)]
mod failure_tests;
#[cfg(test)]
mod proxy_stream_tests;
#[cfg(test)]
mod range_origin_tests;
#[cfg(test)]
mod rejection_tests;
#[cfg(test)]
mod resume_tests;
#[cfg(test)]
mod route_tests;
#[cfg(test)]
mod slots_tests;
#[cfg(test)]
mod waiters_tests;
#[cfg(test)]
mod stand_tests;
#[cfg(test)]
mod readahead_tests;
