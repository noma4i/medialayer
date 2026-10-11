use super::{BodyStream, FetchError, Fetcher, Request, Response};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum Payload {
    Bytes(Vec<u8>),
    Filled { len: u64, byte: u8 },
}

impl Payload {
    fn len(&self) -> u64 {
        match self {
            Payload::Bytes(bytes) => bytes.len() as u64,
            Payload::Filled { len, .. } => *len,
        }
    }

    fn slice(&self, start: u64, end: u64) -> Vec<u8> {
        match self {
            Payload::Bytes(bytes) => bytes[start as usize..end as usize].to_vec(),
            Payload::Filled { byte, .. } => vec![*byte; (end - start) as usize],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Script {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub payload: Payload,
    pub chunk_size: usize,
    pub head_delay: Duration,
    pub chunk_delay: Duration,
    pub cut_after: Option<u64>,
    pub hang: bool,
    pub ignore_range: bool,
    pub unsized_body: bool,
    pub ignore_range_after: Option<usize>,
}

impl Script {
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Script::with_payload(Payload::Bytes(body.into()))
    }

    pub fn filled(len: u64) -> Self {
        Script::with_payload(Payload::Filled { len, byte: 0xAB })
    }

    fn with_payload(payload: Payload) -> Self {
        Script {
            status: 200,
            headers: HashMap::new(),
            payload,
            chunk_size: 16 * 1024,
            head_delay: Duration::ZERO,
            chunk_delay: Duration::ZERO,
            cut_after: None,
            hang: false,
            ignore_range: false,
            unsized_body: false,
            ignore_range_after: None,
        }
    }

    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.insert(name.to_string(), value.to_string());
        self
    }

    pub fn chunked(mut self, chunk_size: usize) -> Self {
        self.chunk_size = chunk_size;
        self
    }

    pub fn head_delay(mut self, delay: Duration) -> Self {
        self.head_delay = delay;
        self
    }

    pub fn chunk_delay(mut self, delay: Duration) -> Self {
        self.chunk_delay = delay;
        self
    }

    pub fn cut_after(mut self, bytes: u64) -> Self {
        self.cut_after = Some(bytes);
        self
    }

    pub fn hanging(mut self) -> Self {
        self.hang = true;
        self
    }

    pub fn ignoring_range_after(mut self, ranged_requests: usize) -> Self {
        self.ignore_range_after = Some(ranged_requests);
        self
    }

    pub fn without_length(mut self) -> Self {
        self.unsized_body = true;
        self
    }

    pub fn ignoring_range(mut self) -> Self {
        self.ignore_range = true;
        self
    }
}

struct TrackedChunk {
    data: Vec<u8>,
    live: Arc<AtomicUsize>,
}

impl AsRef<[u8]> for TrackedChunk {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

impl Drop for TrackedChunk {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

struct BodyCursor {
    payload: Payload,
    position: u64,
    end: u64,
    served: u64,
    script_chunk: usize,
    chunk_delay: Duration,
    cut_after: Option<u64>,
    broken: bool,
    live: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    served_total: Arc<AtomicU64>,
}

async fn next_chunk(mut cursor: BodyCursor) -> Option<(Result<Bytes, FetchError>, BodyCursor)> {
    if cursor.broken || cursor.position >= cursor.end {
        return None;
    }
    if !cursor.chunk_delay.is_zero() {
        tokio::time::sleep(cursor.chunk_delay).await;
    }
    if cursor.cut_after.is_some_and(|limit| cursor.served >= limit) {
        cursor.broken = true;
        return Some((Err(FetchError::Network("connection cut".to_string())), cursor));
    }
    let length = (cursor.script_chunk as u64).min(cursor.end - cursor.position);
    let data = cursor.payload.slice(cursor.position, cursor.position + length);
    cursor.position += length;
    cursor.served += length;
    cursor.served_total.fetch_add(length, Ordering::SeqCst);
    let now = cursor.live.fetch_add(1, Ordering::SeqCst) + 1;
    cursor.peak.fetch_max(now, Ordering::SeqCst);
    let chunk = Bytes::from_owner(TrackedChunk { data, live: cursor.live.clone() });
    Some((Ok(chunk), cursor))
}

#[derive(Default)]
pub struct MockFetcher {
    scripts: Mutex<HashMap<String, Script>>,
    offline: AtomicBool,
    calls: AtomicUsize,
    per_url: Mutex<HashMap<String, usize>>,
    requests: Mutex<Vec<Request>>,
    live: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    served_total: Arc<AtomicU64>,
}

impl MockFetcher {
    pub fn new() -> Self {
        MockFetcher::default()
    }

    pub fn route(&self, url: &str, script: Script) {
        self.scripts.lock().unwrap_or_else(PoisonError::into_inner).insert(url.to_string(), script);
    }

    pub fn set_offline(&self, offline: bool) {
        self.offline.store(offline, Ordering::SeqCst);
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn calls_for(&self, url: &str) -> usize {
        self.per_url.lock().unwrap_or_else(PoisonError::into_inner).get(url).copied().unwrap_or(0)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn live_chunks(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }

    pub fn bytes_served(&self) -> u64 {
        self.served_total.load(Ordering::SeqCst)
    }

    pub fn peak_chunks(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    fn record(&self, request: &Request) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.per_url.lock().unwrap_or_else(PoisonError::into_inner).entry(request.url.clone()).or_default() += 1;
        self.requests.lock().unwrap_or_else(PoisonError::into_inner).push(request.clone());
    }

    fn script_for(&self, url: &str) -> Script {
        let found = self.scripts.lock().unwrap_or_else(PoisonError::into_inner).get(url).cloned();
        found.unwrap_or_else(|| Script::ok(b"not found".to_vec()).status(404))
    }
}

#[async_trait]
impl Fetcher for MockFetcher {
    async fn fetch(&self, request: Request) -> Result<Response, FetchError> {
        self.record(&request);
        if self.offline.load(Ordering::SeqCst) {
            return Err(FetchError::Network("offline".to_string()));
        }
        let script = self.script_for(&request.url);
        if !script.head_delay.is_zero() {
            tokio::time::sleep(script.head_delay).await;
        }
        if script.hang {
            std::future::pending::<()>().await;
        }
        let total = script.payload.len();
        let ranged = self.requests().iter().filter(|seen| seen.url == request.url && seen.range.is_some()).count();
        let ignores = script.ignore_range || script.ignore_range_after.is_some_and(|after| ranged > after);
        let mut headers = script.headers.clone();
        let (status, start, end) = match request.range {
            Some(_) if ignores => (script.status, 0, total),
            Some((start, _)) if start >= total => {
                headers.insert("Content-Range".to_string(), format!("bytes */{total}"));
                (416, 0, 0)
            }
            Some((start, last)) if script.status == 200 => {
                let end = last.map_or(total, |last| (last + 1).min(total));
                headers.insert("Content-Range".to_string(), format!("bytes {start}-{}/{total}", end - 1));
                (206, start, end)
            }
            _ => (script.status, 0, total),
        };
        if !script.unsized_body {
            headers.insert("Content-Length".to_string(), (end - start).to_string());
        }
        let cursor = BodyCursor {
            payload: script.payload,
            position: start,
            end,
            served: 0,
            script_chunk: script.chunk_size,
            chunk_delay: script.chunk_delay,
            cut_after: script.cut_after,
            broken: false,
            live: self.live.clone(),
            peak: self.peak.clone(),
            served_total: self.served_total.clone(),
        };
        let body: BodyStream = Box::pin(stream::unfold(cursor, next_chunk));
        Ok(Response { status, headers, body })
    }
}
