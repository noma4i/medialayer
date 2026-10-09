mod buffer;
mod byte_store;
mod config;
mod flight;
mod inner;
mod route;
mod serve;
mod server;
mod slots;
mod tasks;
mod cursor;

#[cfg(test)]
mod memory_store;
#[cfg(test)]
pub(crate) mod test_client;

pub use byte_store::{ByteStore, Meta, StoreWriter};
pub use config::Config;
pub use route::block_key;
pub use slots::Priority;

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

const FETCH_ATTEMPTS: usize = 3;
const PING_REQUEST: &str = "GET /ping HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
const PING_REPLY_LIMIT: u64 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Stopped,
    Ready,
    Unhealthy,
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
    pub async fn start(config: Config, fetcher: Arc<dyn Fetcher>, store: Arc<dyn ByteStore>) -> Result<Proxy, ProxyError> {
        let tasks = tasks::Tasks::default();
        let inner = Arc::new(ProxyShared::new(config, fetcher, store, tasks.clone()));
        let proxy = Proxy { inner, _owner: Arc::new(tasks::TasksOwner(tasks)) };
        proxy.restart(0).await?;
        Ok(proxy)
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
        } else if self.inner.alive.load(Ordering::SeqCst) && self.port() > 0 {
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
        self.inner.set_timeout(Duration::from_secs(limits.timeout_seconds.into()));
        self.inner.slots.set_max(limits.max_concurrent as usize);
        self.inner.image_slots.set_max(limits.max_concurrent as usize);
    }

    pub fn key_for(&self, url: &str, headers: &HashMap<String, String>) -> String {
        self.inner.key(url, &self.inner.upstream_headers(url, headers))
    }

    pub async fn fetch_bytes(&self, url: &str, headers: &HashMap<String, String>, limit: u64) -> Result<Vec<u8>, ProxyError> {
        self.fetch_into_store(url, headers, Priority::Image).await?;
        let key = self.key_for(url, headers);
        match self.inner.store.len(&key).await {
            Some(length) if length > limit => {
                self.inner.store.remove(&key).await;
                Err(ProxyError::Fetch(format!("response of {length} bytes exceeds the limit of {limit}")))
            }
            _ => self.read_stored(url, headers, limit).await.ok_or_else(|| ProxyError::Fetch("the response vanished from the cache".to_string())),
        }
    }

    pub async fn read_stored(&self, url: &str, headers: &HashMap<String, String>, limit: u64) -> Option<Vec<u8>> {
        let key = self.key_for(url, headers);
        let length = self.inner.store.len(&key).await.filter(|length| *length <= limit)?;
        let _held = self.inner.buffers().reserve_exact(length as usize).await.ok()?;
        self.inner.store.get_range(&key, 0, length).await
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

    pub async fn stop(&self) {
        let mut listener = self.inner.listener.lock().await;
        self.shut_down(&mut listener).await;
    }

    pub async fn restart(&self, preferred_port: u16) -> Result<u16, ProxyError> {
        let mut listener = self.inner.listener.lock().await;
        self.shut_down(&mut listener).await;
        let (started, port) = server::spawn_listener(&self.inner, preferred_port).await?;
        *listener = Some(started);
        let previous = self.inner.last_port.swap(port, Ordering::SeqCst);
        if previous != 0 && previous != port {
            self.inner.route_generation.fetch_add(1, Ordering::SeqCst);
        }
        self.inner.port.store(port, Ordering::SeqCst);
        self.inner.running.store(true, Ordering::SeqCst);
        Ok(port)
    }

    pub async fn fetch_into_store(
        &self,
        url: &str,
        headers: &HashMap<String, String>,
        priority: Priority,
    ) -> Result<(), ProxyError> {
        let upstream = self.inner.upstream_headers(url, headers);
        let key = self.inner.key(url, &upstream);
        self.download_into_store(&key, || self.inner.get_or_start(url, upstream.clone(), priority)).await
    }

    pub async fn fetch_block_into_store(
        &self,
        url: &str,
        block_index: u64,
        headers: &HashMap<String, String>,
        priority: Priority,
    ) -> Result<(), ProxyError> {
        let upstream = self.inner.upstream_headers(url, headers);
        let key = route::block_key(&self.inner.key(url, &upstream), block_index);
        self.download_into_store(&key, || self.inner.get_or_start_block(url, block_index, None, upstream.clone(), priority))
            .await
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
        start: impl Fn() -> flight::FlightSubscription,
    ) -> Result<(), ProxyError> {
        for _ in 0..FETCH_ATTEMPTS {
            if self.inner.store.contains(key).await {
                return Ok(());
            }
            match start().wait_finished().await.outcome {
                FlightOutcome::Complete => return Ok(()),
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

#[cfg(test)]
mod buffer_tests;
#[cfg(test)]
mod lanes_tests;
#[cfg(test)]
mod limits_tests;
#[cfg(test)]
mod namespace_tests;
#[cfg(test)]
mod proxy_tests;
#[cfg(test)]
mod resume_tests;
#[cfg(test)]
mod route_tests;
#[cfg(test)]
mod slots_tests;
#[cfg(test)]
mod stand_tests;
