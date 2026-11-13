use super::cursor::{next_store, Resume, StoreCursor};
use super::flight::{FlightOutcome, Rejection};
use super::inner::ProxyShared;
use super::route::{block_key, content_type_for, resolve_range, ByteRange, RangeOutcome};
use super::slots::Priority;
use bytes::Bytes;
use futures_util::stream;
use hyper::Response;
use std::io;
use std::sync::Arc;
use super::serve_reply::*;
use super::serve::stored_response;
use super::serve_origin::*;

pub(super) enum OpenFailure {
    Rejected(Arc<Rejection>),
    Failed(String),
    StoredWhole,
}

pub(super) struct BlockCursor {
    pub(super) inner: Arc<ProxyShared>,
    pub(super) target: String,
    pub(super) key: String,
    pub(super) headers: Upstream,
    pub(super) total: u64,
    pub(super) position: u64,
    pub(super) end: u64,
    pub(super) block_end: u64,
    pub(super) source: Option<StoreCursor>,
    pub(super) finished: bool,
}

impl BlockCursor {
    pub(super) async fn open(&mut self) -> Result<Option<String>, OpenFailure> {
        let size = self.inner.config.chunk_size;
        let block = self.position / size;
        let block_start = block * size;
        self.block_end = self.end.min(block_start + size).min(self.total);
        let stored_key = block_key(&self.key, block);
        let offset = self.position - block_start;
        let offset_end = self.block_end - block_start;
        let chunk = self.inner.config.stream_chunk as u64;
        let mut cursor = StoreCursor {
            reader: None,
            subscription: None,
            store: self.inner.store.clone(),
            key: stored_key.clone(),
            position: offset,
            end: Some(offset_end),
            chunk,
            finished: false,
            resumed: false,
            resume: Resume::new(&self.inner, &self.target, &self.headers, block_start),
            pending: Bytes::new(),
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
                (FlightOutcome::StoredWhole, _) => return Err(OpenFailure::StoredWhole),
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

pub(super) async fn next_block(mut cursor: BlockCursor) -> Option<(Result<Bytes, io::Error>, BlockCursor)> {
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

pub(super) fn fail_blocks(mut cursor: BlockCursor) -> (Result<Bytes, io::Error>, BlockCursor) {
    cursor.finished = true;
    (Err(io::Error::other("block download failed")), cursor)
}

pub(super) async fn block_range_response(
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
        Err(OpenFailure::StoredWhole) => return None,
        Err(OpenFailure::Failed(_)) if inner.ignores_range(&inner.key(target, &headers)) => return None,
        Err(OpenFailure::Failed(message)) => return Some(text(502, &message)),
    };
    let content_type = content_type_for(target, origin_type.as_deref());
    let response_headers = media_headers(&content_type, end - start + 1, Some(format!("bytes {start}-{end}/{total}")));
    Some(respond(206, response_headers, streamed(stream::unfold(cursor, next_block)), false))
}
