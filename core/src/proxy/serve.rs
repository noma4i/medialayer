use super::flight::{collect_limited, Outcome, Rejection, Subscription};
use super::inner::Inner;
use super::route::{
    absolutize_playlist, block_key, content_type_for, parse_range, resolve_range, target_url, ByteRange, RangeOutcome,
};
use super::slots::{Permit, Priority};
use crate::fetch::{header_value, BodyStream, FetchError, Request};
use crate::hls::{is_manifest_url, rewrite_playlist};
use crate::identity::cache_key;
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
use std::time::Duration;

const ATTEMPTS: usize = 3;
const PLAYLIST_TYPE: &str = "application/vnd.apple.mpegurl";
const PLAYLIST_END_TAG: &str = "#EXT-X-ENDLIST";

pub(super) type ProxyBody = UnsyncBoxBody<Bytes, io::Error>;

type Headers = Vec<(&'static str, String)>;
type Upstream = HashMap<String, String>;
type Failure = Box<Response<ProxyBody>>;

enum Step {
    Done(Response<ProxyBody>),
    Retry,
}

pub(super) async fn handle(inner: Arc<Inner>, request: hyper::Request<Incoming>) -> Response<ProxyBody> {
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
    if is_manifest_url(&target) {
        playlist(&inner, &target, headers, head_only).await
    } else {
        media(&inner, &target, headers, range, head_only).await
    }
}

async fn playlist(inner: &Arc<Inner>, target: &str, headers: Upstream, head_only: bool) -> Response<ProxyBody> {
    let key = cache_key(target);
    for _ in 0..ATTEMPTS {
        if let Some(length) = inner.store.len(&key).await {
            match stored_playlist(inner, &key, length).await {
                Ok(Some(bytes)) => return playlist_response(target, &bytes, head_only),
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
            Outcome::Rejected(rejection) => return rejected(&rejection, head_only),
            Outcome::Failed(message) => return text(502, &message),
            Outcome::Unstorable => return playlist_direct(inner, target, headers, head_only).await,
            _ => {}
        }
    }
    text(503, "playlist unavailable")
}

async fn stored_playlist(inner: &Inner, key: &str, length: u64) -> Result<Option<Vec<u8>>, Failure> {
    if length > inner.config.max_playlist_bytes {
        return Err(Box::new(text(502, "playlist too large")));
    }
    let Some(bytes) = inner.store.get_range(key, 0, length).await else {
        return Err(Box::new(text(502, "playlist vanished from the store")));
    };
    let finished = String::from_utf8_lossy(&bytes).contains(PLAYLIST_END_TAG);
    Ok((finished || inner.playlist_is_fresh(key)).then_some(bytes))
}

async fn playlist_direct(inner: &Inner, target: &str, headers: Upstream, head_only: bool) -> Response<ProxyBody> {
    let (response, _permit) = match origin_fetch(inner, target, headers, None).await {
        Ok(fetched) => fetched,
        Err(failure) => return *failure,
    };
    if response.status != 200 {
        return text(response.status, "origin refused the playlist");
    }
    let limit = inner.config.max_playlist_bytes as usize;
    match collect_limited(response.body, limit, inner.config.policy.timeout).await {
        Some(bytes) => playlist_response(target, &bytes, head_only),
        None => text(502, "playlist body failed"),
    }
}

fn playlist_response(target: &str, bytes: &[u8], head_only: bool) -> Response<ProxyBody> {
    let source = String::from_utf8_lossy(bytes);
    let body = rewrite_playlist(&absolutize_playlist(&source, target));
    let headers = vec![("content-type", PLAYLIST_TYPE.to_string()), ("content-length", body.len().to_string())];
    respond(200, headers, full(Bytes::from(body)), head_only)
}

async fn media(
    inner: &Arc<Inner>,
    target: &str,
    headers: Upstream,
    range: Option<ByteRange>,
    head_only: bool,
) -> Response<ProxyBody> {
    let key = cache_key(target);
    for _ in 0..ATTEMPTS {
        let mut joined = inner.existing_flight(target);
        if joined.is_none() {
            if let Some(total) = inner.store.len(&key).await {
                return stored_response(inner, target, &key, total, range, head_only).await;
            }
            if head_only {
                return head_from_origin(inner, target, headers).await;
            }
            if needs_blocks(range) {
                return block_range_response(inner, target, headers, range).await;
            }
        }
        let subscription = joined.take().unwrap_or_else(|| inner.get_or_start(target, headers.clone(), Priority::Playback));
        match flight_response(inner, subscription, target, &headers, range, head_only).await {
            Step::Done(response) => return response,
            Step::Retry => {}
        }
    }
    text(503, "media unavailable")
}

fn needs_blocks(range: Option<ByteRange>) -> bool {
    matches!(range, Some(ByteRange::Suffix(_)) | Some(ByteRange::From { start: 1.., .. }))
}

async fn stored_response(
    inner: &Inner,
    target: &str,
    key: &str,
    total: u64,
    range: Option<ByteRange>,
    head_only: bool,
) -> Response<ProxyBody> {
    let meta = inner.store.meta(key).await.unwrap_or_default();
    let content_type = content_type_for(target, meta.content_type.as_deref());
    let body = |start: u64, end: u64| {
        let cursor = StoredCursor { store: inner.store.clone(), key: key.to_string(), position: start, end, chunk: inner.config.stream_chunk as u64 };
        streamed(stream::unfold(cursor, next_stored))
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
    inner: &Inner,
    mut subscription: Subscription,
    target: &str,
    headers: &Upstream,
    range: Option<ByteRange>,
    head_only: bool,
) -> Step {
    let progress = subscription.wait_started().await;
    let head = match (progress.outcome, progress.head) {
        (Outcome::Rejected(rejection), _) => return Step::Done(rejected(&rejection, head_only)),
        (Outcome::Failed(message), _) => return Step::Done(text(502, &message)),
        (Outcome::Unstorable, _) => {
            return Step::Done(direct_response(inner, target, headers.clone(), range, head_only).await)
        }
        (Outcome::Cancelled | Outcome::QueueFull, _) | (_, None) => return Step::Retry,
        (_, Some(head)) => head,
    };
    let content_type = content_type_for(target, head.content_type.as_deref());
    let cursor = |position: u64, end: Option<u64>| FlightCursor {
        subscription,
        store: inner.store.clone(),
        key: cache_key(target),
        position,
        end,
        chunk: inner.config.stream_chunk as u64,
        finished: false,
    };
    let response = match head.total {
        None => respond(200, vec![("content-type", content_type), ("accept-ranges", "bytes".to_string())], streamed(stream::unfold(cursor(0, None), next_flight)), head_only),
        Some(total) => match resolve_range(range, total) {
            RangeOutcome::Full => {
                respond(200, media_headers(&content_type, total, None), streamed(stream::unfold(cursor(0, Some(total)), next_flight)), head_only)
            }
            RangeOutcome::Partial { start, end } => {
                let headers = media_headers(&content_type, end - start + 1, Some(format!("bytes {start}-{end}/{total}")));
                respond(206, headers, streamed(stream::unfold(cursor(start, Some(end + 1)), next_flight)), head_only)
            }
            RangeOutcome::Unsatisfiable => unsatisfiable(total),
        },
    };
    Step::Done(response)
}

async fn direct_response(
    inner: &Inner,
    target: &str,
    headers: Upstream,
    range: Option<ByteRange>,
    head_only: bool,
) -> Response<ProxyBody> {
    let upstream_range = match range {
        None => None,
        Some(ByteRange::From { start, end }) => Some((start, end)),
        Some(ByteRange::Suffix(length)) => match probe_total(inner, target, headers.clone()).await {
            Ok((total, _)) if total > 0 => Some((total.saturating_sub(length), Some(total - 1))),
            Ok(_) => return unsatisfiable(0),
            Err(failure) => return *failure,
        },
    };
    let (response, permit) = match origin_fetch(inner, target, headers, upstream_range).await {
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
    let cursor = DirectCursor { body: response.body, _permit: permit, idle: inner.config.policy.timeout, done: false };
    respond(response.status, forwarded, streamed(stream::unfold(cursor, next_direct)), head_only)
}

async fn head_from_origin(inner: &Inner, target: &str, headers: Upstream) -> Response<ProxyBody> {
    match probe_total(inner, target, headers).await {
        Ok((total, origin_type)) => {
            let content_type = content_type_for(target, origin_type.as_deref());
            respond(200, media_headers(&content_type, total, None), empty(), false)
        }
        Err(failure) => *failure,
    }
}

async fn probe_total(inner: &Inner, target: &str, headers: Upstream) -> Result<(u64, Option<String>), Failure> {
    let (response, _permit) = origin_fetch(inner, target, headers, Some((0, Some(0)))).await?;
    let total = header_value(&response.headers, "content-range")
        .and_then(|value| value.rsplit_once('/'))
        .and_then(|(_, total)| total.trim().parse::<u64>().ok())
        .or_else(|| {
            let whole = response.status == 200;
            header_value(&response.headers, "content-length").filter(|_| whole).and_then(|value| value.trim().parse().ok())
        });
    let origin_type = header_value(&response.headers, "content-type").map(str::to_string);
    match (response.status, total) {
        (200 | 206, Some(total)) => Ok((total, origin_type)),
        (200 | 206, None) => Err(Box::new(text(502, "origin did not report a size"))),
        (status, _) => Err(Box::new(text(status, "origin refused the request"))),
    }
}

async fn origin_fetch(
    inner: &Inner,
    target: &str,
    headers: Upstream,
    range: Option<(u64, Option<u64>)>,
) -> Result<(crate::fetch::Response, Permit), Failure> {
    let Ok(permit) = inner.slots.acquire(Priority::Playback).await else {
        return Err(Box::new(text(503, "busy")));
    };
    let request = Request { url: target.to_string(), headers, range };
    match tokio::time::timeout(inner.config.policy.timeout, inner.fetcher.fetch(request)).await {
        Err(_) => Err(Box::new(text(502, "origin timed out"))),
        Ok(Err(FetchError::Status(status))) => Err(Box::new(text(status, "origin refused the request"))),
        Ok(Err(error)) => Err(Box::new(text(502, &error.to_string()))),
        Ok(Ok(response)) => Ok((response, permit)),
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

struct StoredCursor {
    store: Arc<dyn super::byte_store::ByteStore>,
    key: String,
    position: u64,
    end: u64,
    chunk: u64,
}

async fn next_stored(mut cursor: StoredCursor) -> Option<(Result<Bytes, io::Error>, StoredCursor)> {
    if cursor.position >= cursor.end {
        return None;
    }
    let wanted = cursor.chunk.min(cursor.end - cursor.position);
    match cursor.store.get_range(&cursor.key, cursor.position, wanted).await {
        Some(data) if !data.is_empty() => {
            cursor.position += data.len() as u64;
            Some((Ok(Bytes::from(data)), cursor))
        }
        _ => {
            cursor.position = cursor.end;
            Some((Err(io::Error::new(io::ErrorKind::UnexpectedEof, "store entry vanished")), cursor))
        }
    }
}

struct FlightCursor {
    subscription: Subscription,
    store: Arc<dyn super::byte_store::ByteStore>,
    key: String,
    position: u64,
    end: Option<u64>,
    chunk: u64,
    finished: bool,
}

async fn next_flight(mut cursor: FlightCursor) -> Option<(Result<Bytes, io::Error>, FlightCursor)> {
    loop {
        if cursor.finished || cursor.end.is_some_and(|end| cursor.position >= end) {
            return None;
        }
        let progress = cursor.subscription.snapshot();
        if cursor.position < progress.written {
            let mut wanted = cursor.chunk.min(progress.written - cursor.position);
            if let Some(end) = cursor.end {
                wanted = wanted.min(end - cursor.position);
            }
            match cursor.store.get_range(&cursor.key, cursor.position, wanted).await {
                Some(data) if !data.is_empty() => {
                    cursor.position += data.len() as u64;
                    return Some((Ok(Bytes::from(data)), cursor));
                }
                _ => return Some(abort(cursor)),
            }
        }
        match progress.outcome {
            Outcome::Complete => return None,
            Outcome::Running => {
                if !cursor.subscription.changed().await {
                    return Some(abort(cursor));
                }
            }
            _ => return Some(abort(cursor)),
        }
    }
}

fn abort(mut cursor: FlightCursor) -> (Result<Bytes, io::Error>, FlightCursor) {
    cursor.finished = true;
    (Err(io::Error::other("download failed")), cursor)
}

struct DirectCursor {
    body: BodyStream,
    _permit: Permit,
    idle: Duration,
    done: bool,
}

async fn next_direct(mut cursor: DirectCursor) -> Option<(Result<Bytes, io::Error>, DirectCursor)> {
    if cursor.done {
        return None;
    }
    match tokio::time::timeout(cursor.idle, cursor.body.next()).await {
        Ok(Some(Ok(chunk))) => Some((Ok(chunk), cursor)),
        Ok(None) => None,
        _ => {
            cursor.done = true;
            Some((Err(io::Error::other("origin body failed")), cursor))
        }
    }
}

enum Source {
    Stored(StoredCursor),
    Flight(FlightCursor),
}

enum OpenFailure {
    Rejected(Arc<Rejection>),
    Failed(String),
    Unstorable,
}

struct BlockCursor {
    inner: Arc<Inner>,
    target: String,
    key: String,
    headers: Upstream,
    total: u64,
    position: u64,
    end: u64,
    block_end: u64,
    source: Option<Source>,
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
        for _ in 0..ATTEMPTS {
            if self.inner.store.len(&stored_key).await.is_some() {
                let content_type = self.inner.store.meta(&stored_key).await.and_then(|meta| meta.content_type);
                let cursor = StoredCursor {
                    store: self.inner.store.clone(),
                    key: stored_key,
                    position: offset,
                    end: offset_end,
                    chunk,
                };
                self.source = Some(Source::Stored(cursor));
                return Ok(content_type);
            }
            let mut subscription =
                self.inner.get_or_start_block(&self.target, block, self.total, self.headers.clone(), Priority::Playback);
            let progress = subscription.wait_started().await;
            match (progress.outcome, progress.head) {
                (Outcome::Rejected(rejection), _) => return Err(OpenFailure::Rejected(rejection)),
                (Outcome::Failed(message), _) => return Err(OpenFailure::Failed(message)),
                (Outcome::Unstorable, _) => return Err(OpenFailure::Unstorable),
                (Outcome::Cancelled | Outcome::QueueFull, _) | (_, None) => {}
                (_, Some(head)) => {
                    let cursor = FlightCursor {
                        subscription,
                        store: self.inner.store.clone(),
                        key: stored_key,
                        position: offset,
                        end: Some(offset_end),
                        chunk,
                        finished: false,
                    };
                    self.source = Some(Source::Flight(cursor));
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
        let step = match source {
            Source::Stored(inner_cursor) => next_stored(inner_cursor).await.map(|(item, rest)| (item, Source::Stored(rest))),
            Source::Flight(inner_cursor) => next_flight(inner_cursor).await.map(|(item, rest)| (item, Source::Flight(rest))),
        };
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
    inner: &Arc<Inner>,
    target: &str,
    headers: Upstream,
    range: Option<ByteRange>,
) -> Response<ProxyBody> {
    let key = cache_key(target);
    let total = match known_total(inner, target, &key, &headers, range).await {
        Ok(total) => total,
        Err(failure) => return *failure,
    };
    let (start, end) = match resolve_range(range, total) {
        RangeOutcome::Partial { start, end } => (start, end),
        _ => return unsatisfiable(total),
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
        Err(OpenFailure::Rejected(rejection)) => return rejected(&rejection, false),
        Err(OpenFailure::Failed(message)) => return text(502, &message),
        Err(OpenFailure::Unstorable) => {
            return direct_response(inner, target, headers, range, false).await;
        }
    };
    let content_type = content_type_for(target, origin_type.as_deref());
    let response_headers = media_headers(&content_type, end - start + 1, Some(format!("bytes {start}-{end}/{total}")));
    respond(206, response_headers, streamed(stream::unfold(cursor, next_block)), false)
}

async fn known_total(
    inner: &Inner,
    target: &str,
    key: &str,
    headers: &Upstream,
    range: Option<ByteRange>,
) -> Result<u64, Failure> {
    if let Some(total) = inner.known_total(key) {
        return Ok(total);
    }
    if let Some(ByteRange::From { start, .. }) = range {
        let first = block_key(key, start / inner.config.chunk_size);
        if let Some(total) = inner.store.meta(&first).await.and_then(|meta| meta.total_length) {
            inner.remember_total(key, total);
            return Ok(total);
        }
    }
    let (total, _) = probe_total(inner, target, headers.clone()).await?;
    inner.remember_total(key, total);
    Ok(total)
}
