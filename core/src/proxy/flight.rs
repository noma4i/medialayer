use super::inner::{ParkedWriter, ProxyShared};
use super::route::{answered_length, content_range_total};
use super::slots::Priority;
use crate::fetch::{header_value, BodyStream, FetchError, Request};
use super::buffer::Buffers;
use super::byte_store::{Meta, StoreWriter};
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
pub(super) enum FlightOutcome {
    Running,
    Complete,
    Rejected(Arc<Rejection>),
    Unstorable,
    QueueFull,
    Failed(String),
    Cancelled,
    StoredWhole,
}

#[derive(Debug, Clone)]
pub(super) struct FlightProgress {
    pub head: Option<Arc<Head>>,
    pub written: u64,
    pub outcome: FlightOutcome,
}

impl FlightProgress {
    pub fn is_running(&self) -> bool {
        matches!(self.outcome, FlightOutcome::Running)
    }
}

pub(super) struct Flight {
    pub url: String,
    pub key: String,
    pub base_key: String,
    pub range: Option<(u64, u64)>,
    progress: watch::Sender<FlightProgress>,
    subscribers: AtomicUsize,
    cancel: Notify,
    forced: AtomicBool,
    promoted: AtomicBool,
    promote: Notify,
}

impl Flight {
    pub fn new(url: &str, key: &str, base_key: &str, range: Option<(u64, u64)>, priority: Priority) -> Arc<Flight> {
        let (progress, _) = watch::channel(FlightProgress { head: None, written: 0, outcome: FlightOutcome::Running });
        Arc::new(Flight {
            url: url.to_string(),
            key: key.to_string(),
            base_key: base_key.to_string(),
            range,
            progress,
            subscribers: AtomicUsize::new(0),
            cancel: Notify::new(),
            forced: AtomicBool::new(false),
            promoted: AtomicBool::new(!priority.is_prefetch()),
            promote: Notify::new(),
        })
    }

    pub fn written(&self) -> u64 {
        self.progress.borrow().written
    }

    pub fn failure(&self, outcome: &FlightOutcome) -> Option<crate::failures::MediaFailure> {
        use crate::failures::{FailureStage, MediaFailure};
        let (stage, reason) = match outcome {
            FlightOutcome::Failed(message) => (FailureStage::Load, message.clone()),
            FlightOutcome::Rejected(rejection) => (FailureStage::Load, format!("origin answered status {}", rejection.status)),
            FlightOutcome::Unstorable => (FailureStage::Store, "the cache refused the write".to_string()),
            _ => return None,
        };
        let content_type = self.progress.borrow().head.as_ref().and_then(|head| head.content_type.clone());
        let kind = crate::cache::classify_kind(&self.url, &Meta { content_type, total_length: None }).into();
        Some(MediaFailure { kind, url: self.url.clone(), stage, reason })
    }

    pub fn promote(&self) {
        if !self.promoted.swap(true, Ordering::SeqCst) {
            self.promote.notify_one();
        }
    }

    pub fn subscribe(self: &Arc<Flight>) -> FlightSubscription {
        self.subscribers.fetch_add(1, Ordering::SeqCst);
        FlightSubscription { flight: self.clone(), receiver: self.progress.subscribe(), lingering: false }
    }

    pub fn cancel(&self) {
        self.cancel.notify_one();
    }

    pub fn force_cancel(&self) {
        self.forced.store(true, Ordering::SeqCst);
        self.cancel.notify_one();
    }

    pub fn is_abandoned(&self) -> bool {
        self.forced.load(Ordering::SeqCst) || self.subscribers.load(Ordering::SeqCst) == 0
    }

    pub fn finish(&self, outcome: FlightOutcome) {
        self.progress.send_modify(|progress| progress.outcome = outcome);
    }

    fn publish_head(&self, head: Head) {
        self.progress.send_modify(|progress| progress.head = Some(Arc::new(head)));
    }

    fn publish_written(&self, written: u64) {
        self.progress.send_modify(|progress| progress.written = written);
    }
}

pub(super) struct FlightSubscription {
    flight: Arc<Flight>,
    receiver: watch::Receiver<FlightProgress>,
    lingering: bool,
}

impl FlightSubscription {
    pub fn linger(&mut self) {
        self.lingering = true;
    }

    pub fn snapshot(&mut self) -> FlightProgress {
        self.receiver.borrow_and_update().clone()
    }

    pub async fn changed(&mut self) -> bool {
        self.receiver.changed().await.is_ok()
    }

    pub async fn wait_started(&mut self) -> FlightProgress {
        self.wait(|progress| progress.head.is_some() || !progress.is_running()).await
    }

    pub async fn wait_finished(&mut self, progress: Option<&Notify>) -> FlightProgress {
        let mut written = 0;
        loop {
            let current = self.snapshot();
            if current.written > written {
                written = current.written;
                if let Some(progress) = progress { progress.notify_one(); }
            }
            if !current.is_running() || !self.changed().await {
                return current;
            }
        }
    }

    async fn wait(&mut self, ready: impl FnMut(&FlightProgress) -> bool) -> FlightProgress {
        match self.receiver.wait_for(ready).await {
            Ok(progress) => progress.clone(),
            Err(_) => FlightProgress { head: None, written: 0, outcome: FlightOutcome::Failed("download vanished".to_string()) },
        }
    }
}

impl Drop for FlightSubscription {
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
                if chunk.len() > limit - collected.len() {
                    return None;
                }
                if chunk.len() > collected.capacity() - collected.len() {
                    let grown = (collected.capacity() * 2).max(collected.len() + chunk.len()).min(limit);
                    collected.reserve_exact(grown - collected.len());
                }
                collected.extend_from_slice(&chunk);
            }
            Ok(None) => return Some(collected),
            _ => return None,
        }
    }
}

async fn wait_retired(inner: &ProxyShared, flight: &Arc<Flight>) {
    loop {
        flight.cancel.notified().await;
        if inner.retire_if_abandoned(flight) {
            return;
        }
    }
}

async fn cancellable<T>(inner: &ProxyShared, flight: &Arc<Flight>, future: impl Future<Output = T>) -> Option<T> {
    tokio::select! {
        biased;
        _ = wait_retired(inner, flight) => None,
        value = future => Some(value),
    }
}

async fn keep(inner: &ProxyShared, key: &str, writer: Box<dyn StoreWriter>, written: u64, parks: bool) {
    if parks && written > 0 {
        crate::trace!("flight", "park {} at {written}", crate::trace::tail(key));
        inner.park(key, ParkedWriter { writer, written });
    } else {
        writer.abort().await;
    }
}

pub(super) async fn download(inner: &ProxyShared, flight: &Arc<Flight>, headers: HashMap<String, String>, priority: Priority) -> FlightOutcome {
    if inner.store.contains(&flight.key).await {
        return FlightOutcome::Complete;
    }
    let mut priority = priority;
    let queued = std::time::Instant::now();
    let deadline = tokio::time::Instant::now() + inner.config.permit_wait;
    let permit = loop {
        tokio::select! {
            biased;
            _ = wait_retired(inner, flight) => return FlightOutcome::Cancelled,
            _ = tokio::time::sleep_until(deadline) => {
                crate::trace!("heal", "capacity permit-timeout");
                return FlightOutcome::Rejected(Arc::new(Rejection { status: 503, content_type: None, body: Bytes::new() }));
            }
            _ = flight.promote.notified(), if priority.is_prefetch() => priority = priority.promoted(),
            permit = inner.gate(priority).acquire(priority) => match permit {
                Ok(permit) => break permit,
                Err(QueueFull) => return FlightOutcome::QueueFull,
            },
        }
    };
    crate::trace!("flight", "permit {} waited={}ms {priority:?}", crate::trace::tail(&flight.url), queued.elapsed().as_millis());
    let timeout = inner.timeout();
    let buffers = inner.buffers();
    let parked = match flight.range {
        Some((start, end)) if inner.parked_len(&flight.key) < end + 1 - start => inner.parked_len(&flight.key),
        _ => 0,
    };
    let request = Request { url: flight.url.clone(), headers, range: flight.range.map(|(start, end)| (start + parked, Some(end))) };
    let fetched = match cancellable(inner, flight, tokio::time::timeout(timeout, inner.fetcher.fetch(request))).await {
        None => return FlightOutcome::Cancelled,
        Some(Err(_)) => return FlightOutcome::Failed("origin timed out".to_string()),
        Some(Ok(Err(FetchError::Status(status)))) => {
            return FlightOutcome::Rejected(Arc::new(Rejection { status, content_type: None, body: Bytes::new() }))
        }
        Some(Ok(Err(error))) => return FlightOutcome::Failed(error.to_string()),
        Some(Ok(Ok(response))) => crate::fetch::Response { body: permit.hold(response.body), ..response },
    };
    let mut range = flight.range;
    let mut key = &flight.key;
    let mut whole_body = false;
    let expected = if range.is_some() { 206 } else { 200 };
    if range.is_some() && fetched.status == 200 {
        drop(inner.take_parked(&flight.key));
        inner.mark_ignores_range(&flight.base_key);
        range = None;
        key = &flight.base_key;
        whole_body = true;
    } else if fetched.status != expected {
        let content_type = header_value(&fetched.headers, "content-type").map(str::to_string);
        let Some(collected) = cancellable(inner, flight, collect_limited(fetched.body, REJECTION_BODY_LIMIT, timeout, &buffers)).await else {
            return FlightOutcome::Cancelled;
        };
        let body = collected.unwrap_or_default();
        return FlightOutcome::Rejected(Arc::new(Rejection { status: fetched.status, content_type, body: Bytes::from(body) }));
    }
    let total = header_value(&fetched.headers, "content-length").and_then(|value| value.trim().parse::<u64>().ok());
    let content_type = header_value(&fetched.headers, "content-type").map(str::to_string);
    let object_length = match range {
        Some(_) => content_range_total(&fetched.headers),
        None => total,
    };
    if let Some(length) = object_length.filter(|_| range.is_some()) {
        inner.remember_total(&flight.base_key, length);
    }
    let parked = if range.is_some() { parked } else { 0 };
    let expected = match range {
        Some((start, end)) => match answered_length(&fetched.headers, start + parked, Some(end)) {
            Ok(length) => length.map(|length| length + parked),
            Err(message) => return FlightOutcome::Failed(message),
        },
        None => total,
    };
    let total = total.map(|total| total + parked);
    let meta = Meta { content_type: content_type.clone(), total_length: object_length };
    let mut writer = if parked > 0 {
        match inner.take_parked(key) {
            Some(held) if held.written == parked => held.writer,
            _ => return FlightOutcome::Failed("the parked part of the block vanished".to_string()),
        }
    } else {
        let Some(writer) = inner.store.begin_write(key, meta).await else {
            return FlightOutcome::Unstorable;
        };
        writer
    };
    let parks = range.is_some();
    if !whole_body {
        flight.publish_head(Head { total, content_type });
    }

    let mut body = fetched.body;
    let mut written = parked;
    if parked > 0 {
        flight.publish_written(written);
    }
    loop {
        let _buffer = match cancellable(inner, flight, buffers.reserve_chunk()).await {
            None => {
                keep(inner, key, writer, written, parks).await;
                return FlightOutcome::Cancelled;
            }
            Some(Err(_)) => {
                keep(inner, key, writer, written, parks).await;
                return FlightOutcome::Failed("no memory for the download".to_string());
            }
            Some(Ok(buffer)) => buffer,
        };
        let next = match cancellable(inner, flight, tokio::time::timeout(timeout, body.next())).await {
            None => {
                keep(inner, key, writer, written, parks).await;
                return FlightOutcome::Cancelled;
            }
            Some(Err(_)) => {
                keep(inner, key, writer, written, parks).await;
                return FlightOutcome::Failed("origin stalled".to_string());
            }
            Some(Ok(next)) => next,
        };
        match next {
            None => break,
            Some(Err(error)) => {
                keep(inner, key, writer, written, parks).await;
                return FlightOutcome::Failed(error.to_string());
            }
            Some(Ok(chunk)) => {
                if !writer.append(&chunk).await {
                    writer.abort().await;
                    return FlightOutcome::Unstorable;
                }
                written += chunk.len() as u64;
                flight.publish_written(written);
                if expected == Some(written) {
                    break;
                }
            }
        }
    }
    drop(body);
    if expected.is_some_and(|expected| expected != written) {
        keep(inner, key, writer, written, parks).await;
        return FlightOutcome::Failed("origin body truncated".to_string());
    }
    let committed = match expected {
        Some(length) => writer.commit_exact(length).await,
        None => writer.commit().await,
    };
    if committed && whole_body {
        FlightOutcome::StoredWhole
    } else if committed {
        FlightOutcome::Complete
    } else {
        FlightOutcome::Unstorable
    }
}
