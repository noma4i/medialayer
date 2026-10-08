use super::buffer::Buffers;
use super::byte_store::ByteStore;
use super::flight::{Outcome, Subscription};
use super::inner::Inner;
use super::serve::origin_fetch;
use super::slots::Permit;
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
    pub _permit: Permit,
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
    pub inner: Arc<Inner>,
    pub target: String,
    pub headers: HashMap<String, String>,
    pub origin_offset: u64,
    direct: Option<DirectCursor>,
}

impl Resume {
    pub fn new(inner: &Arc<Inner>, target: &str, headers: &HashMap<String, String>, origin_offset: u64) -> Self {
        Resume { inner: inner.clone(), target: target.to_string(), headers: headers.clone(), origin_offset, direct: None }
    }

    async fn next(&mut self, position: &mut u64, end: Option<u64>) -> Option<ChunkResult> {
        if self.direct.is_none() {
            let start = self.origin_offset + *position;
            let range = (start > 0 || end.is_some()).then(|| (start, end.map(|end| self.origin_offset + end - 1)));
            let Ok((response, permit)) = origin_fetch(&self.inner, &self.target, self.headers.clone(), range).await else {
                return Some(Err(io::Error::other("origin unavailable for the resume")));
            };
            if response.status != if range.is_some() { 206 } else { 200 } {
                return Some(Err(io::Error::other("origin refused the resume")));
            }
            let idle = self.inner.timeout();
            self.direct = Some(DirectCursor { body: response.body, _permit: permit, idle, buffers: self.inner.buffers(), done: false });
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
    pub subscription: Option<Subscription>,
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
            None => (cursor.end.unwrap_or(0), Outcome::Complete),
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
            match cursor.store.get_range(&cursor.key, cursor.position, wanted).await {
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
            Outcome::Complete => return None,
            Outcome::Unstorable => cursor.resumed = true,
            Outcome::Running => {
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
