use crate::api::MediaError;
use crate::fetch::{self, BodyStream, FetchError};
use crate::memory::{bounded, InputLimits};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream::{self, Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FetchRequest {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub range_start: Option<u64>,
    pub range_end: Option<u64>,
}

#[derive(uniffi::Record)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Arc<dyn BodyReader>,
}

#[uniffi::export(with_foreign)]
#[async_trait]
pub trait BodyReader: Send + Sync {
    async fn next_chunk(&self) -> Result<Option<Vec<u8>>, MediaError>;
    fn release(&self);
}

#[uniffi::export(with_foreign)]
#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, MediaError>;
}

pub struct ForeignFetcher {
    fetcher: Arc<dyn Fetcher>,
    limits: InputLimits,
}

impl ForeignFetcher {
    pub fn new(fetcher: Arc<dyn Fetcher>, limits: InputLimits) -> ForeignFetcher {
        ForeignFetcher { fetcher, limits }
    }
}

#[async_trait]
impl fetch::Fetcher for ForeignFetcher {
    async fn fetch(&self, request: fetch::Request) -> Result<fetch::Response, FetchError> {
        let (range_start, range_end) = request.range.map_or((None, None), |(start, end)| (Some(start), end));
        let request = FetchRequest { url: request.url, headers: request.headers, range_start, range_end };
        let response = self.fetcher.fetch(request).await.map_err(network)?;
        Ok(fetch::Response { status: response.status, headers: response.headers, body: body_stream(response.body, self.limits) })
    }
}

fn network(error: MediaError) -> FetchError {
    FetchError::Network(error.to_string())
}

struct ClosingReader(Arc<dyn BodyReader>);

impl Drop for ClosingReader {
    fn drop(&mut self) {
        let reader = self.0.clone();
        if crate::proxy::closing_inline() {
            return reader.release();
        }
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn_blocking(move || reader.release());
            }
            Err(_) => reader.release(),
        }
    }
}

struct ClosingBody {
    chunks: BodyStream,
    _release: ClosingReader,
}

impl Stream for ClosingBody {
    type Item = Result<Bytes, FetchError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.chunks.as_mut().poll_next(context)
    }
}

fn body_stream(reader: Arc<dyn BodyReader>, limits: InputLimits) -> BodyStream {
    let release = ClosingReader(reader.clone());
    let chunks = stream::unfold(Some(reader), move |state| async move {
        let reader = state?;
        match reader.next_chunk().await {
            Ok(Some(chunk)) => match bounded(chunk.len() as u64, limits.response_bytes) {
                Ok(()) => Some((Ok(Bytes::from(chunk)), Some(reader))),
                Err(error) => Some((Err(FetchError::Network(error.to_string())), None)),
            },
            Ok(None) => None,
            Err(error) => Some((Err(network(error)), None)),
        }
    })
    .boxed();
    Box::pin(ClosingBody { chunks, _release: release })
}
