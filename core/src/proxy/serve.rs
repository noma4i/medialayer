use super::cursor::{next_direct, next_store, DirectCursor, Resume, StoreCursor};
use super::flight::{collect_limited, FlightOutcome, Rejection, FlightSubscription};
use super::inner::ProxyShared;
use super::route::{
    absolutize_playlist, block_key, playlist_references, content_type_for, parse_range, resolve_range, target_url, ByteRange, RangeOutcome,
};
use super::slots::Priority;
use crate::fetch::{header_value, FetchError, Request};
use crate::hls::{extract_variants, is_manifest_url, is_multivariant, keep_first_variant, rewrite_playlist};
use crate::memory::Reservation;
use bytes::Bytes;
use futures_util::{stream, Stream, StreamExt};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt, Empty, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::header::{HeaderName, HeaderValue};
use hyper::{Method, Response, StatusCode};
use std::collections::HashMap;
use std::io;
use std::sync::Arc;

const ATTEMPTS: usize = 3;
const PLAYLIST_TYPE: &str = "application/vnd.apple.mpegurl";
const PLAYLIST_END_TAG: &str = "#EXT-X-ENDLIST";
const SEGMENT_TAG: &str = "#EXTINF";

#[cfg(test)]
#[path = "probe_tests.rs"]
mod probe_tests;

pub(super) type ProxyBody = UnsyncBoxBody<Bytes, io::Error>;

type Headers = Vec<(&'static str, String)>;
type Upstream = HashMap<String, String>;
type Failure = Box<Response<ProxyBody>>;

enum Step {
    Done(Response<ProxyBody>),
    Retry,
}

pub(super) async fn handle(inner: Arc<ProxyShared>, request: hyper::Request<Incoming>) -> Response<ProxyBody> {
    let head_only = request.method() == Method::HEAD;
    if request.method() != Method::GET && !head_only {
        return text(405, "method not allowed");
    }
    if request.uri().path() == "/ping" {
        return text(200, "pong");
    }
    let Some(target) = target_url(request.uri().path(), request.uri().query()) else {
        return text(404, "unknown route");
    };
    let incoming: Upstream = request
        .headers()
        .iter()
        .filter_map(|(name, value)| value.to_str().ok().map(|value| (name.as_str().to_string(), value.to_string())))
        .collect();
    let range = header_value(&incoming, "range").and_then(parse_range);
    let headers = inner.upstream_headers(&target, &incoming);
    let exchange = Exchange::open(&inner, &target, header_value(&incoming, "range"), head_only);
    let response = if is_manifest_url(&target) {
        playlist(&inner, &target, headers, head_only).await
    } else {
        media(&inner, &target, headers, range, head_only).await
    };
    Exchange::answer(exchange, response)
}

struct Exchange {
    label: String,
    started: std::time::Instant,
    bytes: u64,
    expected: Option<u64>,
}

impl Exchange {
    fn open(inner: &ProxyShared, target: &str, range: Option<&str>, head_only: bool) -> Option<Exchange> {
        if !crate::trace::enabled() {
            return None;
        }
        let label = format!("{} range={} head={head_only}", crate::trace::tail(target), range.unwrap_or("-"));
        crate::trace!("proxy", "request {label} live={} slots={}", inner.live.load(std::sync::atomic::Ordering::SeqCst), inner.slots.in_use());
        Some(Exchange { label, started: std::time::Instant::now(), bytes: 0, expected: None })
    }

    fn count(&mut self, bytes: u64) {
        self.bytes += bytes;
    }

    fn answer(exchange: Option<Exchange>, response: Response<ProxyBody>) -> Response<ProxyBody> {
        let Some(mut exchange) = exchange else { return response };
        let length = |name: &str| response.headers().get(name).and_then(|value| value.to_str().ok()).map(str::to_string);
        exchange.expected = length("content-length").and_then(|value| value.parse().ok());
        crate::trace!(
            "proxy",
            "response {} status={} length={:?} range={:?} after={}ms",
            exchange.label,
            response.status().as_u16(),
            exchange.expected,
            length("content-range"),
            exchange.started.elapsed().as_millis()
        );
        let (parts, body) = response.into_parts();
        let body = body
            .map_frame(move |frame| {
                exchange.count(frame.data_ref().map_or(0, |data| data.len() as u64));
                frame
            })
            .boxed_unsync();
        Response::from_parts(parts, body)
    }
}

impl Drop for Exchange {
    fn drop(&mut self) {
        crate::trace!(
            "proxy",
            "closed {} bytes={} of {:?} after={}ms",
            self.label,
            self.bytes,
            self.expected,
            self.started.elapsed().as_millis()
        );
    }
}

async fn playlist(inner: &Arc<ProxyShared>, target: &str, headers: Upstream, head_only: bool) -> Response<ProxyBody> {
    let key = inner.key(target, &headers);
    for _ in 0..ATTEMPTS {
        if let Some(length) = inner.store.len(&key).await {
            match stored_playlist(inner, &key, length).await {
                Ok(Some((bytes, held))) => {
                    let response = playlist_response(inner, target, &bytes, &headers, head_only);
                    drop(bytes);
                    drop(held);
                    return response;
                }
                Ok(None) => {
                    inner.store.remove(&key).await;
                    inner.forget_playlist(&key);
                    continue;
                }
                Err(failure) => return *failure,
            }
        }
        let mut subscription = inner.get_or_start(target, headers.clone(), Priority::Playback);
        match subscription.wait_finished().await.outcome {
            FlightOutcome::Rejected(rejection) => return rejected(&rejection, head_only),
            FlightOutcome::Failed(message) => return text(502, &message),
            FlightOutcome::Unstorable => return playlist_direct(inner, target, headers, head_only).await,
            _ => {}
        }
    }
    text(503, "playlist unavailable")
}

async fn stored_playlist(inner: &ProxyShared, key: &str, length: u64) -> Result<Option<(Vec<u8>, Reservation)>, Failure> {
    if length > inner.config.max_playlist_bytes {
        return Err(Box::new(text(502, "playlist too large")));
    }
    let Ok(held) = inner.buffers().reserve_exact(length as usize).await else {
        return Err(Box::new(text(503, "no memory for the playlist")));
    };
    let Some(bytes) = inner.store.get_range(key, 0, length).await else {
        return Err(Box::new(text(502, "playlist vanished from the store")));
    };
    let text = String::from_utf8_lossy(&bytes);
    let is_static = text.contains(PLAYLIST_END_TAG) || (is_multivariant(&text) && !text.contains(SEGMENT_TAG));
    drop(text);
    Ok((is_static || inner.playlist_is_fresh(key)).then_some((bytes, held)))
}

async fn playlist_direct(inner: &ProxyShared, target: &str, headers: Upstream, head_only: bool) -> Response<ProxyBody> {
    let response = match origin_fetch(inner, target, headers.clone(), None).await {
        Ok(fetched) => fetched,
        Err(failure) => return *failure,
    };
    if response.status != 200 {
        return text(response.status, "origin refused the playlist");
    }
    let limit = inner.config.max_playlist_bytes as usize;
    match collect_limited(response.body, limit, inner.timeout(), &inner.buffers()).await {
        Some(bytes) => playlist_response(inner, target, &bytes, &headers, head_only),
        None => text(502, "playlist body failed"),
    }
}

fn playlist_response(inner: &ProxyShared, target: &str, bytes: &[u8], headers: &Upstream, head_only: bool) -> Response<ProxyBody> {
    let source = String::from_utf8_lossy(bytes);
    let absolute = absolutize_playlist(&source, target);
    inner.inherit_headers(&playlist_references(&absolute), headers);
    let kept = keep_first_variant(&absolute);
    if !head_only {
        match extract_variants(&kept).first() {
            Some(variant) => inner.windows.remember_variant(variant, target),
            None => inner.windows.register(target, &kept),
        }
    }
    let body = rewrite_playlist(&kept);
    let headers = vec![("content-type", PLAYLIST_TYPE.to_string()), ("content-length", body.len().to_string())];
    respond(200, headers, full(Bytes::from(body)), head_only)
}

async fn media(
    inner: &Arc<ProxyShared>,
    target: &str,
    headers: Upstream,
    range: Option<ByteRange>,
    head_only: bool,
) -> Response<ProxyBody> {
    let key = inner.key(target, &headers);
    if !head_only {
        inner.hold_for_playhead(target, &key).await;
    }
    for _ in 0..ATTEMPTS {
        let mut joined = inner.existing_flight(target, &headers);
        if joined.is_none() {
            if let Some(total) = inner.store.len(&key).await {
                return stored_response(inner, target, &key, total, range, &headers, head_only).await;
            }
            if head_only {
                return head_from_origin(inner, target, headers).await;
            }
            let blocks = needs_blocks(range) || first_block_is_stored(inner, &key, range).await;
            if blocks && !inner.ignores_range(&key) {
                if let Some(response) = block_range_response(inner, target, headers.clone(), range).await {
                    return response;
                }
            }
        }
        let mut subscription = joined.take().unwrap_or_else(|| inner.get_or_start(target, headers.clone(), Priority::Playback));
        if inner.ignores_range(&key) {
            subscription.linger();
        }
        match flight_response(inner, subscription, target, &headers, range, head_only).await {
            Step::Done(response) => return response,
            Step::Retry => {}
        }
    }
    text(503, "media unavailable")
}

async fn first_block_is_stored(inner: &ProxyShared, key: &str, range: Option<ByteRange>) -> bool {
    matches!(range, Some(ByteRange::From { start: 0, end: Some(_) })) && inner.store.len(&block_key(key, 0)).await.is_some()
}

fn needs_blocks(range: Option<ByteRange>) -> bool {
    matches!(range, Some(ByteRange::Suffix(_)) | Some(ByteRange::From { start: 1.., .. }))
}

async fn stored_response(
    inner: &Arc<ProxyShared>,
    target: &str,
    key: &str,
    total: u64,
    range: Option<ByteRange>,
    headers: &Upstream,
    head_only: bool,
) -> Response<ProxyBody> {
    let meta = inner.store.meta(key).await.unwrap_or_default();
    let content_type = content_type_for(target, meta.content_type.as_deref());
    let body = |start: u64, end: u64| {
        let cursor = StoreCursor {
            subscription: None,
            store: inner.store.clone(),
            key: key.to_string(),
            position: start,
            end: Some(end),
            chunk: inner.config.stream_chunk as u64,
            finished: false,
            resumed: false,
            resume: Resume::new(inner, target, headers, 0),
        };
        streamed(stream::unfold(cursor, next_store))
    };
    match resolve_range(range, total) {
        RangeOutcome::Full => {
            let headers = media_headers(&content_type, total, None);
            respond(200, headers, body(0, total), head_only)
        }
        RangeOutcome::Partial { start, end } => {
            let headers = media_headers(&content_type, end - start + 1, Some(format!("bytes {start}-{end}/{total}")));
            respond(206, headers, body(start, end + 1), head_only)
        }
        RangeOutcome::Unsatisfiable => unsatisfiable(total),
    }
}

async fn flight_response(
    inner: &Arc<ProxyShared>,
    mut subscription: FlightSubscription,
    target: &str,
    headers: &Upstream,
    range: Option<ByteRange>,
    head_only: bool,
) -> Step {
    let mut progress = subscription.wait_started().await;
    if range.is_some() && progress.head.as_ref().is_some_and(|head| head.total.is_none()) {
        progress = subscription.wait_finished().await;
        if matches!(progress.outcome, FlightOutcome::Complete) {
            return Step::Retry;
        }
    }
    let head = match (progress.outcome, progress.head) {
        (FlightOutcome::Rejected(rejection), _) => return Step::Done(rejected(&rejection, head_only)),
        (FlightOutcome::Failed(message), _) => return Step::Done(text(502, &message)),
        (FlightOutcome::Unstorable, _) => {
            return Step::Done(direct_response(inner, target, headers.clone(), range, head_only).await)
        }
        (FlightOutcome::Cancelled | FlightOutcome::QueueFull, _) | (_, None) => return Step::Retry,
        (_, Some(head)) => head,
    };
    let content_type = content_type_for(target, head.content_type.as_deref());
    let cursor = |position: u64, end: Option<u64>| StoreCursor {
        subscription: Some(subscription),
        store: inner.store.clone(),
        key: inner.key(target, headers),
        position,
        end,
        chunk: inner.config.stream_chunk as u64,
        finished: false,
        resumed: false,
        resume: Resume::new(inner, target, headers, 0),
    };
    let response = match head.total {
        None => respond(200, vec![("content-type", content_type), ("accept-ranges", "bytes".to_string())], streamed(stream::unfold(cursor(0, None), next_store)), head_only),
        Some(total) => match resolve_range(range, total) {
            RangeOutcome::Full => {
                respond(200, media_headers(&content_type, total, None), streamed(stream::unfold(cursor(0, Some(total)), next_store)), head_only)
            }
            RangeOutcome::Partial { start, end } => {
                let headers = media_headers(&content_type, end - start + 1, Some(format!("bytes {start}-{end}/{total}")));
                respond(206, headers, streamed(stream::unfold(cursor(start, Some(end + 1)), next_store)), head_only)
            }
            RangeOutcome::Unsatisfiable => unsatisfiable(total),
        },
    };
    Step::Done(response)
}

async fn direct_response(
    inner: &ProxyShared,
    target: &str,
    headers: Upstream,
    range: Option<ByteRange>,
    head_only: bool,
) -> Response<ProxyBody> {
    let upstream_range = match range {
        None => None,
        Some(ByteRange::From { start, end }) => Some((start, end)),
        Some(ByteRange::Suffix(length)) => match probe_sized(inner, target, headers.clone()).await {
            Ok((total, _)) if total > 0 => Some((total.saturating_sub(length), Some(total - 1))),
            Ok(_) => return unsatisfiable(0),
            Err(failure) => return *failure,
        },
    };
    let response = match origin_fetch(inner, target, headers, upstream_range).await {
        Ok(fetched) => fetched,
        Err(failure) => return *failure,
    };
    let origin_type = header_value(&response.headers, "content-type");
    let mut forwarded: Headers = vec![("content-type", content_type_for(target, origin_type))];
    for name in ["content-length", "content-range", "accept-ranges"] {
        if let Some(value) = header_value(&response.headers, name) {
            forwarded.push((name, value.to_string()));
        }
    }
    let cursor = DirectCursor { body: response.body, idle: inner.timeout(), buffers: inner.buffers(), done: false };
    respond(response.status, forwarded, streamed(stream::unfold(cursor, next_direct)), head_only)
}

async fn head_from_origin(inner: &ProxyShared, target: &str, headers: Upstream) -> Response<ProxyBody> {
    match probe_sized(inner, target, headers).await {
        Ok((total, origin_type)) => {
            let content_type = content_type_for(target, origin_type.as_deref());
            respond(200, media_headers(&content_type, total, None), empty(), false)
        }
        Err(failure) => *failure,
    }
}

struct Probe {
    total: Option<u64>,
    origin_type: Option<String>,
    ignored_range: bool,
}

async fn probe_total(inner: &ProxyShared, target: &str, headers: Upstream) -> Result<Probe, Failure> {
    let response = origin_fetch(inner, target, headers, Some((0, Some(0)))).await?;
    let total = header_value(&response.headers, "content-range")
        .and_then(|value| value.rsplit_once('/'))
        .and_then(|(_, total)| total.trim().parse::<u64>().ok())
        .or_else(|| {
            let whole = response.status == 200;
            header_value(&response.headers, "content-length").filter(|_| whole).and_then(|value| value.trim().parse().ok())
        });
    let origin_type = header_value(&response.headers, "content-type").map(str::to_string);
    match (response.status, total) {
        (200, total) => Ok(Probe { total, origin_type, ignored_range: true }),
        (206, Some(total)) => Ok(Probe { total: Some(total), origin_type, ignored_range: false }),
        (206, None) => Err(Box::new(text(502, "origin did not report a size"))),
        (status, _) => Err(Box::new(text(status, "origin refused the request"))),
    }
}

async fn probe_sized(inner: &ProxyShared, target: &str, headers: Upstream) -> Result<(u64, Option<String>), Failure> {
    let probe = probe_total(inner, target, headers).await?;
    match probe.total {
        Some(total) => Ok((total, probe.origin_type)),
        None => Err(Box::new(text(502, "origin did not report a size"))),
    }
}

pub(super) async fn origin_fetch(
    inner: &ProxyShared,
    target: &str,
    headers: Upstream,
    range: Option<(u64, Option<u64>)>,
) -> Result<crate::fetch::Response, Failure> {
    let Ok(permit) = inner.slots.acquire(Priority::Playback).await else {
        return Err(Box::new(text(503, "busy")));
    };
    let request = Request { url: target.to_string(), headers, range };
    crate::trace!("origin", "fetch {} range={range:?}", crate::trace::tail(target));
    match tokio::time::timeout(inner.timeout(), inner.fetcher.fetch(request)).await {
        Err(_) => Err(Box::new(text(502, "origin timed out"))),
        Ok(Err(FetchError::Status(status))) => Err(Box::new(text(status, "origin refused the request"))),
        Ok(Err(error)) => Err(Box::new(text(502, &error.to_string()))),
        Ok(Ok(response)) => Ok(crate::fetch::Response { body: permit.hold(response.body), ..response }),
    }
}

fn media_headers(content_type: &str, length: u64, content_range: Option<String>) -> Headers {
    let mut headers: Headers = vec![
        ("content-type", content_type.to_string()),
        ("accept-ranges", "bytes".to_string()),
        ("content-length", length.to_string()),
    ];
    if let Some(value) = content_range {
        headers.push(("content-range", value));
    }
    headers
}

fn unsatisfiable(total: u64) -> Response<ProxyBody> {
    respond(416, vec![("content-range", format!("bytes */{total}")), ("content-length", "0".to_string())], empty(), false)
}

fn rejected(rejection: &Rejection, head_only: bool) -> Response<ProxyBody> {
    let mut headers: Headers = vec![("content-length", rejection.body.len().to_string())];
    if let Some(content_type) = &rejection.content_type {
        headers.push(("content-type", content_type.clone()));
    }
    respond(rejection.status, headers, full(rejection.body.clone()), head_only)
}

fn text(status: u16, message: &str) -> Response<ProxyBody> {
    let headers = vec![("content-type", "text/plain; charset=utf-8".to_string()), ("content-length", message.len().to_string())];
    respond(status, headers, full(Bytes::from(message.to_string())), false)
}

fn respond(status: u16, headers: Headers, body: ProxyBody, head_only: bool) -> Response<ProxyBody> {
    let mut response = Response::new(if head_only { empty() } else { body });
    *response.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    for (name, value) in headers {
        if let Ok(value) = HeaderValue::from_str(&value) {
            response.headers_mut().insert(HeaderName::from_static(name), value);
        }
    }
    response
}

fn full(bytes: Bytes) -> ProxyBody {
    Full::new(bytes).map_err(|never| match never {}).boxed_unsync()
}

fn empty() -> ProxyBody {
    Empty::<Bytes>::new().map_err(|never| match never {}).boxed_unsync()
}

fn streamed(source: impl Stream<Item = Result<Bytes, io::Error>> + Send + 'static) -> ProxyBody {
    StreamBody::new(source.map(|chunk| chunk.map(Frame::data))).boxed_unsync()
}

enum OpenFailure {
    Rejected(Arc<Rejection>),
    Failed(String),
}

struct BlockCursor {
    inner: Arc<ProxyShared>,
    target: String,
    key: String,
    headers: Upstream,
    total: u64,
    position: u64,
    end: u64,
    block_end: u64,
    source: Option<StoreCursor>,
    finished: bool,
}

impl BlockCursor {
    async fn open(&mut self) -> Result<Option<String>, OpenFailure> {
        let size = self.inner.config.chunk_size;
        let block = self.position / size;
        let block_start = block * size;
        self.block_end = self.end.min(block_start + size).min(self.total);
        let stored_key = block_key(&self.key, block);
        let offset = self.position - block_start;
        let offset_end = self.block_end - block_start;
        let chunk = self.inner.config.stream_chunk as u64;
        let mut cursor = StoreCursor {
            subscription: None,
            store: self.inner.store.clone(),
            key: stored_key.clone(),
            position: offset,
            end: Some(offset_end),
            chunk,
            finished: false,
            resumed: false,
            resume: Resume::new(&self.inner, &self.target, &self.headers, block_start),
        };
        let whole_block = (block_start + size).min(self.total) - block_start;
        for _ in 0..ATTEMPTS {
            let stored = self.inner.store.len(&stored_key).await;
            if stored.is_some_and(|length| length != whole_block) {
                crate::trace!("cache", "bad block {} stored={stored:?} expected={whole_block}", crate::trace::tail(&stored_key));
                self.inner.store.remove(&stored_key).await;
            } else if stored.is_some() {
                let content_type = self.inner.store.meta(&stored_key).await.and_then(|meta| meta.content_type);
                self.source = Some(cursor);
                return Ok(content_type);
            }
            let mut subscription =
                self.inner.get_or_start_block(&self.target, block, Some(self.total), self.headers.clone(), Priority::Playback);
            subscription.linger();
            let progress = subscription.wait_started().await;
            match (progress.outcome, progress.head) {
                (FlightOutcome::Rejected(rejection), _) => return Err(OpenFailure::Rejected(rejection)),
                (FlightOutcome::Failed(message), _) => return Err(OpenFailure::Failed(message)),
                (FlightOutcome::Unstorable, _) => {
                    cursor.resumed = true;
                    self.source = Some(cursor);
                    return Ok(None);
                }
                (FlightOutcome::Cancelled | FlightOutcome::QueueFull, _) | (_, None) => {}
                (_, Some(head)) => {
                    cursor.subscription = Some(subscription);
                    self.source = Some(cursor);
                    return Ok(head.content_type.clone());
                }
            }
        }
        Err(OpenFailure::Failed("block unavailable".to_string()))
    }
}

async fn next_block(mut cursor: BlockCursor) -> Option<(Result<Bytes, io::Error>, BlockCursor)> {
    loop {
        if cursor.finished || cursor.position >= cursor.end {
            return None;
        }
        let Some(source) = cursor.source.take() else {
            if cursor.open().await.is_err() {
                return Some(fail_blocks(cursor));
            }
            continue;
        };
        let step = next_store(source).await;
        match step {
            None if cursor.position < cursor.block_end => return Some(fail_blocks(cursor)),
            None => {}
            Some((Err(error), _)) => {
                cursor.finished = true;
                return Some((Err(error), cursor));
            }
            Some((Ok(bytes), rest)) => {
                cursor.position += bytes.len() as u64;
                cursor.source = Some(rest);
                return Some((Ok(bytes), cursor));
            }
        }
    }
}

fn fail_blocks(mut cursor: BlockCursor) -> (Result<Bytes, io::Error>, BlockCursor) {
    cursor.finished = true;
    (Err(io::Error::other("block download failed")), cursor)
}

async fn block_range_response(
    inner: &Arc<ProxyShared>,
    target: &str,
    headers: Upstream,
    range: Option<ByteRange>,
) -> Option<Response<ProxyBody>> {
    let key = inner.key(target, &headers);
    let total = match known_total(inner, target, &key, &headers, range).await {
        Ok(Some(total)) => total,
        Ok(None) => return None,
        Err(failure) => return Some(*failure),
    };
    if let Some(length) = inner.store.len(&key).await {
        return Some(stored_response(inner, target, &key, length, range, &headers, false).await);
    }
    let (start, end) = match resolve_range(range, total) {
        RangeOutcome::Partial { start, end } => (start, end),
        _ => return Some(unsatisfiable(total)),
    };
    let mut cursor = BlockCursor {
        inner: inner.clone(),
        target: target.to_string(),
        key,
        headers: headers.clone(),
        total,
        position: start,
        end: end + 1,
        block_end: start,
        source: None,
        finished: false,
    };
    let origin_type = match cursor.open().await {
        Ok(content_type) => content_type,
        Err(OpenFailure::Rejected(rejection)) => return Some(rejected(&rejection, false)),
        Err(OpenFailure::Failed(_)) if inner.ignores_range(&inner.key(target, &headers)) => return None,
        Err(OpenFailure::Failed(message)) => return Some(text(502, &message)),
    };
    let content_type = content_type_for(target, origin_type.as_deref());
    let response_headers = media_headers(&content_type, end - start + 1, Some(format!("bytes {start}-{end}/{total}")));
    Some(respond(206, response_headers, streamed(stream::unfold(cursor, next_block)), false))
}

async fn known_total(
    inner: &ProxyShared,
    target: &str,
    key: &str,
    headers: &Upstream,
    range: Option<ByteRange>,
) -> Result<Option<u64>, Failure> {
    if let Some(total) = inner.known_total(key) {
        return Ok(Some(total));
    }
    if let Some(ByteRange::From { start, .. }) = range {
        let first = block_key(key, start / inner.config.chunk_size);
        if let Some(total) = inner.store.meta(&first).await.and_then(|meta| meta.total_length) {
            inner.remember_total(key, total);
            return Ok(Some(total));
        }
    }
    let lock = inner.probe_lock(key);
    probe_once(inner, target, key, headers, &lock.lock).await
}

async fn probe_once(
    inner: &ProxyShared,
    target: &str,
    key: &str,
    headers: &Upstream,
    lock: &tokio::sync::Mutex<()>,
) -> Result<Option<u64>, Failure> {
    let _turn = lock.lock().await;
    if inner.ignores_range(key) {
        return Ok(None);
    }
    if let Some(total) = inner.known_total(key) {
        return Ok(Some(total));
    }
    let probe = probe_total(inner, target, headers.clone()).await?;
    if probe.ignored_range {
        inner.mark_ignores_range(key);
        return Ok(None);
    }
    Ok(probe.total.inspect(|total| inner.remember_total(key, *total)))
}
