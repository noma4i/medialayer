use super::buffer::Buffers;
use super::byte_store::ByteStore;
use super::flight::{FlightOutcome, FlightSubscription};
use super::inner::ProxyShared;
use super::serve::origin_fetch;
use crate::fetch::BodyStream;
use bytes::Bytes;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::time::Duration;

pub(super) type ChunkResult = Result<Bytes, io::Error>;

pub(super) struct DirectCursor {
    pub body: BodyStream,
    pub idle: Duration,
    pub buffers: Buffers,
    pub done: bool,
}

pub(super) async fn next_direct(mut cursor: DirectCursor) -> Option<(ChunkResult, DirectCursor)> {
    if cursor.done {
        return None;
    }
    let Ok(reservation) = cursor.buffers.reserve_chunk().await else {
        cursor.done = true;
        return Some((Err(io::Error::other("no memory for the response")), cursor));
    };
    match tokio::time::timeout(cursor.idle, cursor.body.next()).await {
        Ok(Some(Ok(chunk))) => Some((Ok(Buffers::hold(chunk, reservation)), cursor)),
        Ok(None) => None,
        _ => {
            cursor.done = true;
            Some((Err(io::Error::other("origin body failed")), cursor))
        }
    }
}

pub(super) struct Resume {
    pub inner: Arc<ProxyShared>,
    pub target: String,
    pub headers: HashMap<String, String>,
    pub origin_offset: u64,
    direct: Option<DirectCursor>,
}

impl Resume {
    pub fn new(inner: &Arc<ProxyShared>, target: &str, headers: &HashMap<String, String>, origin_offset: u64) -> Self {
        Resume { inner: inner.clone(), target: target.to_string(), headers: headers.clone(), origin_offset, direct: None }
    }

    async fn next(&mut self, position: &mut u64, end: Option<u64>) -> Option<ChunkResult> {
        if self.direct.is_none() {
            let start = self.origin_offset + *position;
            let range = (start > 0 || end.is_some()).then(|| (start, end.map(|end| self.origin_offset + end - 1)));
            let Ok(response) = origin_fetch(&self.inner, &self.target, self.headers.clone(), range).await else {
                return Some(Err(io::Error::other("origin unavailable for the resume")));
            };
            if response.status != if range.is_some() { 206 } else { 200 } {
                return Some(Err(io::Error::other("origin refused the resume")));
            }
            let idle = self.inner.timeout();
            self.direct = Some(DirectCursor { body: response.body, idle, buffers: self.inner.buffers(), done: false });
        }
        let cursor = self.direct.take()?;
        let (item, cursor) = next_direct(cursor).await?;
        if let Ok(bytes) = &item {
            *position += bytes.len() as u64;
        }
        self.direct = Some(cursor);
        Some(item)
    }
}

pub(super) struct StoreCursor {
    pub reader: Option<Box<dyn super::StoreReader>>,
    pub subscription: Option<FlightSubscription>,
    pub store: Arc<dyn ByteStore>,
    pub key: String,
    pub position: u64,
    pub end: Option<u64>,
    pub chunk: u64,
    pub finished: bool,
    pub resumed: bool,
    pub resume: Resume,
}

pub(super) async fn next_store(mut cursor: StoreCursor) -> Option<(ChunkResult, StoreCursor)> {
    loop {
        if cursor.finished || cursor.end.is_some_and(|end| cursor.position >= end) {
            return None;
        }
        if cursor.resumed {
            let item = cursor.resume.next(&mut cursor.position, cursor.end).await?;
            return Some((item, cursor));
        }
        let (written, outcome) = match cursor.subscription.as_mut() {
            Some(subscription) => {
                let progress = subscription.snapshot();
                (progress.written, progress.outcome)
            }
            None => (cursor.end.unwrap_or(0), FlightOutcome::Complete),
        };
        if cursor.position < written {
            let mut wanted = cursor.chunk.min(written - cursor.position);
            if let Some(end) = cursor.end {
                wanted = wanted.min(end - cursor.position);
            }
            let buffers = cursor.resume.inner.buffers();
            let Ok(reservation) = buffers.reserve_between(buffers.floor().min(wanted as usize), wanted as usize).await else {
                return Some(fail_memory(cursor));
            };
            let wanted = reservation.bytes().min(wanted);
            if cursor.reader.is_none() {
                cursor.reader = cursor.store.open_reader(&cursor.key).await;
            }
            let data = match cursor.reader.as_mut() {
                Some(reader) => reader.get_range(cursor.position, wanted).await,
                None => None,
            };
            match data {
                Some(data) if !data.is_empty() => {
                    cursor.position += data.len() as u64;
                    return Some((Ok(Buffers::hold(Bytes::from(data), reservation)), cursor));
                }
                _ => {
                    cursor.resumed = true;
                    continue;
                }
            }
        }
        match outcome {
            FlightOutcome::Complete => return None,
            FlightOutcome::Unstorable => cursor.resumed = true,
            FlightOutcome::Running => {
                let alive = match cursor.subscription.as_mut() {
                    Some(subscription) => subscription.changed().await,
                    None => false,
                };
                if !alive {
                    return Some(fail(cursor));
                }
            }
            _ => return Some(fail(cursor)),
        }
    }
}

fn fail_memory(mut cursor: StoreCursor) -> (ChunkResult, StoreCursor) {
    cursor.finished = true;
    (Err(io::Error::other("no memory for the response")), cursor)
}

fn fail(mut cursor: StoreCursor) -> (ChunkResult, StoreCursor) {
    cursor.finished = true;
    (Err(io::Error::other("download failed")), cursor)
}
