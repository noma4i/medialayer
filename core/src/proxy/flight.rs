use super::inner::Inner;
use super::slots::Priority;
use crate::fetch::{header_value, BodyStream, FetchError, Request};
use super::buffer::Buffers;
use super::byte_store::Meta;
use super::slots::QueueFull;
use bytes::Bytes;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Notify};

const REJECTION_BODY_LIMIT: usize = 64 * 1024;

#[derive(Debug)]
pub(super) struct Head {
    pub total: Option<u64>,
    pub content_type: Option<String>,
}

#[derive(Debug)]
pub(super) struct Rejection {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Bytes,
}

#[derive(Debug, Clone)]
pub(super) enum Outcome {
    Running,
    Complete,
    Rejected(Arc<Rejection>),
    Unstorable,
    QueueFull,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub(super) struct Progress {
    pub head: Option<Arc<Head>>,
    pub written: u64,
    pub outcome: Outcome,
}

impl Progress {
    pub fn is_running(&self) -> bool {
        matches!(self.outcome, Outcome::Running)
    }
}

pub(super) struct Flight {
    pub url: String,
    pub key: String,
    pub base_key: String,
    pub range: Option<(u64, u64)>,
    progress: watch::Sender<Progress>,
    subscribers: AtomicUsize,
    cancel: Notify,
    promoted: AtomicBool,
    promote: Notify,
}

impl Flight {
    pub fn new(url: &str, key: &str, base_key: &str, range: Option<(u64, u64)>, priority: Priority) -> Arc<Flight> {
        let (progress, _) = watch::channel(Progress { head: None, written: 0, outcome: Outcome::Running });
        Arc::new(Flight {
            url: url.to_string(),
            key: key.to_string(),
            base_key: base_key.to_string(),
            range,
            progress,
            subscribers: AtomicUsize::new(0),
            cancel: Notify::new(),
            promoted: AtomicBool::new(priority == Priority::Playback),
            promote: Notify::new(),
        })
    }

    pub fn promote(&self) {
        if !self.promoted.swap(true, Ordering::SeqCst) {
            self.promote.notify_one();
        }
    }

    pub fn subscribe(self: &Arc<Flight>) -> Subscription {
        self.subscribers.fetch_add(1, Ordering::SeqCst);
        Subscription { flight: self.clone(), receiver: self.progress.subscribe(), lingering: false }
    }

    pub fn cancel(&self) {
        self.cancel.notify_one();
    }

    pub fn finish(&self, outcome: Outcome) {
        self.progress.send_modify(|progress| progress.outcome = outcome);
    }

    fn publish_head(&self, head: Head) {
        self.progress.send_modify(|progress| progress.head = Some(Arc::new(head)));
    }

    fn publish_written(&self, written: u64) {
        self.progress.send_modify(|progress| progress.written = written);
    }
}

pub(super) struct Subscription {
    flight: Arc<Flight>,
    receiver: watch::Receiver<Progress>,
    lingering: bool,
}

impl Subscription {
    pub fn linger(&mut self) {
        self.lingering = true;
    }

    pub fn snapshot(&mut self) -> Progress {
        self.receiver.borrow_and_update().clone()
    }

    pub async fn changed(&mut self) -> bool {
        self.receiver.changed().await.is_ok()
    }

    pub async fn wait_started(&mut self) -> Progress {
        self.wait(|progress| progress.head.is_some() || !progress.is_running()).await
    }

    pub async fn wait_finished(&mut self) -> Progress {
        self.wait(|progress| !progress.is_running()).await
    }

    async fn wait(&mut self, ready: impl FnMut(&Progress) -> bool) -> Progress {
        match self.receiver.wait_for(ready).await {
            Ok(progress) => progress.clone(),
            Err(_) => Progress { head: None, written: 0, outcome: Outcome::Failed("download vanished".to_string()) },
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let last = self.flight.subscribers.fetch_sub(1, Ordering::SeqCst) == 1;
        if last && !self.lingering && self.receiver.borrow().is_running() {
            self.flight.cancel();
        }
    }
}

pub(super) async fn collect_limited(mut body: BodyStream, limit: usize, idle: Duration, buffers: &Buffers) -> Option<Vec<u8>> {
    let _held = buffers.reserve_exact(limit).await.ok()?;
    let mut collected = Vec::new();
    loop {
        match tokio::time::timeout(idle, body.next()).await {
            Ok(Some(Ok(chunk))) => {
                collected.extend_from_slice(&chunk);
                if collected.len() > limit {
                    return None;
                }
            }
            Ok(None) => return Some(collected),
            _ => return None,
        }
    }
}

async fn cancellable<T>(cancel: &Notify, future: impl Future<Output = T>) -> Option<T> {
    tokio::select! {
        biased;
        _ = cancel.notified() => None,
        value = future => Some(value),
    }
}

pub(super) async fn download(inner: &Inner, flight: &Flight, headers: HashMap<String, String>, priority: Priority) -> Outcome {
    if inner.store.contains(&flight.key).await {
        return Outcome::Complete;
    }
    let mut priority = priority;
    let permit = loop {
        tokio::select! {
            biased;
            _ = flight.cancel.notified() => return Outcome::Cancelled,
            _ = flight.promote.notified(), if priority == Priority::Prefetch => priority = Priority::Playback,
            permit = inner.slots.acquire(priority) => match permit {
                Ok(permit) => break permit,
                Err(QueueFull) => return Outcome::QueueFull,
            },
        }
    };
    let timeout = inner.timeout();
    let buffers = inner.buffers();
    let request = Request { url: flight.url.clone(), headers, range: flight.range.map(|(start, end)| (start, Some(end))) };
    let fetched = match cancellable(&flight.cancel, tokio::time::timeout(timeout, inner.fetcher.fetch(request))).await {
        None => return Outcome::Cancelled,
        Some(Err(_)) => return Outcome::Failed("origin timed out".to_string()),
        Some(Ok(Err(FetchError::Status(status)))) => {
            return Outcome::Rejected(Arc::new(Rejection { status, content_type: None, body: Bytes::new() }))
        }
        Some(Ok(Err(error))) => return Outcome::Failed(error.to_string()),
        Some(Ok(Ok(response))) => response,
    };
    let expected = if flight.range.is_some() { 206 } else { 200 };
    if fetched.status != expected {
        if flight.range.is_some() && fetched.status == 200 {
            return Outcome::Failed("origin ignored the range".to_string());
        }
        let content_type = header_value(&fetched.headers, "content-type").map(str::to_string);
        let body = collect_limited(fetched.body, REJECTION_BODY_LIMIT, timeout, &buffers).await.unwrap_or_default();
        return Outcome::Rejected(Arc::new(Rejection { status: fetched.status, content_type, body: Bytes::from(body) }));
    }
    let total = header_value(&fetched.headers, "content-length").and_then(|value| value.trim().parse::<u64>().ok());
    let content_type = header_value(&fetched.headers, "content-type").map(str::to_string);
    let object_length = match flight.range {
        Some(_) => header_value(&fetched.headers, "content-range")
            .and_then(|value| value.rsplit_once('/'))
            .and_then(|(_, length)| length.trim().parse::<u64>().ok()),
        None => total,
    };
    if let Some(length) = object_length.filter(|_| flight.range.is_some()) {
        inner.remember_total(&flight.base_key, length);
    }
    let meta = Meta { content_type: content_type.clone(), total_length: object_length };
    let Some(mut writer) = inner.store.begin_write(&flight.key, meta).await else {
        return Outcome::Unstorable;
    };
    flight.publish_head(Head { total, content_type });

    let mut body = fetched.body;
    let mut written = 0u64;
    loop {
        let _buffer = match cancellable(&flight.cancel, buffers.reserve_chunk()).await {
            None => {
                writer.abort().await;
                return Outcome::Cancelled;
            }
            Some(Err(_)) => {
                writer.abort().await;
                return Outcome::Failed("no memory for the download".to_string());
            }
            Some(Ok(buffer)) => buffer,
        };
        let next = match cancellable(&flight.cancel, tokio::time::timeout(timeout, body.next())).await {
            None => {
                writer.abort().await;
                return Outcome::Cancelled;
            }
            Some(Err(_)) => {
                writer.abort().await;
                return Outcome::Failed("origin stalled".to_string());
            }
            Some(Ok(next)) => next,
        };
        match next {
            None => break,
            Some(Err(error)) => {
                writer.abort().await;
                return Outcome::Failed(error.to_string());
            }
            Some(Ok(chunk)) => {
                if !writer.append(&chunk).await {
                    writer.abort().await;
                    return Outcome::Unstorable;
                }
                written += chunk.len() as u64;
                flight.publish_written(written);
            }
        }
    }
    drop(permit);
    if total.is_some_and(|expected| expected != written) {
        writer.abort().await;
        return Outcome::Failed("origin body truncated".to_string());
    }
    if writer.commit().await {
        Outcome::Complete
    } else {
        Outcome::Unstorable
    }
}
