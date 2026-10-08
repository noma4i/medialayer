mod byte_store;
mod config;
mod flight;
mod inner;
mod route;
mod serve;
mod server;
mod slots;

#[cfg(test)]
mod memory_store;
#[cfg(test)]
mod test_client;

pub use byte_store::{ByteStore, Meta, StoreWriter};
pub use config::Config;
pub use slots::Priority;

use crate::fetch::Fetcher;
use crate::identity::cache_key;
use flight::Outcome;
use inner::Inner;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
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
    inner: Arc<Inner>,
}

impl Proxy {
    pub async fn start(config: Config, fetcher: Arc<dyn Fetcher>, store: Arc<dyn ByteStore>) -> Result<Proxy, ProxyError> {
        let proxy = Proxy { inner: Arc::new(Inner::new(config, fetcher, store)) };
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
        for _ in 0..FETCH_ATTEMPTS {
            if self.inner.store.contains(&cache_key(url)).await {
                return Ok(());
            }
            let mut subscription = self.inner.get_or_start(url, upstream.clone(), priority);
            match subscription.wait_finished().await.outcome {
                Outcome::Complete => return Ok(()),
                Outcome::Rejected(rejection) => return Err(ProxyError::Status(rejection.status)),
                Outcome::Failed(message) => return Err(ProxyError::Fetch(message)),
                Outcome::Unstorable => return Err(ProxyError::StoreRefused),
                Outcome::QueueFull => return Err(ProxyError::QueueFull),
                Outcome::Cancelled | Outcome::Running => {}
            }
        }
        Err(ProxyError::Fetch("download cancelled".to_string()))
    }

    async fn shut_down(&self, listener: &mut Option<inner::Listener>) {
        self.inner.running.store(false, Ordering::SeqCst);
        self.inner.port.store(0, Ordering::SeqCst);
        let Some(stopped) = listener.take() else { return };
        let _ = stopped.shutdown.send(true);
        let _ = stopped.task.await;
        self.inner.cancel_all_flights();
    }
}

#[cfg(test)]
mod proxy_tests;
#[cfg(test)]
mod route_tests;
#[cfg(test)]
mod slots_tests;
