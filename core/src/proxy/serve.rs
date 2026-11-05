use http_body_util::BodyExt;
use super::cursor::{next_direct, next_store, DirectCursor, Resume, StoreCursor};
use super::flight::{collect_limited, FlightOutcome, FlightSubscription};
use super::inner::ProxyShared;
use super::route::{answered_length, block_key, content_type_for, parse_range, resolve_range, target_url, ByteRange, RangeOutcome};
use super::playlists::StoredPlaylist;
use super::slots::Priority;
use crate::fetch::header_value;
use crate::hls::{is_manifest_url, Playlist};
use bytes::Bytes;
use futures_util::stream;
use hyper::body::Incoming;
use hyper::{Method, Response};
use std::sync::Arc;
use super::serve_blocks::*;
use super::serve_origin::*;
use super::serve_reply::*;

const PLAYLIST_TYPE: &str = "application/vnd.apple.mpegurl";

#[cfg(test)]
#[path = "probe_tests.rs"]
mod probe_tests;

#[cfg(test)]
#[path = "playlist_work_tests.rs"]
mod playlist_work_tests;

#[cfg(test)]
#[path = "register_tests.rs"]
mod register_tests;


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
        if inner.store.contains(&key).await {
            match inner.stored_plan(target, &key).await {
                Ok(Some(stored)) => {
                    if !inner.windows.restart_registered(target, &stored, &headers) {
                        register_plan(inner, target, &stored.plan, &headers, head_only, Some(&stored));
                    }
                    let body = stored.body();
                    return respond(200, vec![("content-type", PLAYLIST_TYPE.to_string()), ("content-length", body.len().to_string())], full(body), head_only);
                }
                Ok(None) => {
                    inner.store.remove(&key).await;
                    inner.forget_playlist(&key);
                    continue;
                }
                Err(message) => return text(502, message),
            }
        }
        let mut subscription = inner.get_or_start(target, headers.clone(), Priority::Playback);
        match subscription.wait_finished(None).await.outcome {
            FlightOutcome::Rejected(rejection) => return rejected(&rejection, head_only),
            FlightOutcome::Failed(message) => return text(502, &message),
            FlightOutcome::Unstorable => return playlist_direct(inner, target, headers, head_only).await,
            _ => {}
        }
    }
    text(503, "playlist unavailable")
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
    let plan = Playlist::parse(&source, target);
    register_plan(inner, target, &plan, headers, head_only, None);
    let body = plan.body;
    let headers = vec![("content-type", PLAYLIST_TYPE.to_string()), ("content-length", body.len().to_string())];
    respond(200, headers, full(body), head_only)
}

fn register_plan(inner: &ProxyShared, target: &str, plan: &Playlist, headers: &Upstream, head_only: bool, stored: Option<&Arc<StoredPlaylist>>) {
    inner.inherit_headers(&plan.references, headers);
    if !head_only {
        match plan.variant.as_ref() {
            Some(variant) => inner.windows.remember_variant(variant, target),
            None => match stored {
                Some(stored) => inner.windows.register_stored(target, stored, headers),
                None => inner.windows.register_plan(target, plan),
            },
        }
    }
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

pub(super) async fn stored_response(
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
            reader: None,
            subscription: None,
            store: inner.store.clone(),
            key: key.to_string(),
            position: start,
            end: Some(end),
            chunk: inner.config.stream_chunk as u64,
            finished: false,
            resumed: false,
            resume: Resume::new(inner, target, headers, 0),
            pending: Bytes::new(),
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
        progress = subscription.wait_finished(None).await;
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
        reader: None,
        subscription: Some(subscription),
        store: inner.store.clone(),
        key: inner.key(target, headers),
        position,
        end,
        chunk: inner.config.stream_chunk as u64,
        finished: false,
        resumed: false,
        resume: Resume::new(inner, target, headers, 0),
        pending: Bytes::new(),
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
    if let (Some((start, end)), 206) = (upstream_range, response.status) {
        if let Err(message) = answered_length(&response.headers, start, end) {
            return text(502, &message);
        }
    }
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
