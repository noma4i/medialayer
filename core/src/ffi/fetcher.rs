use crate::api::MediaError;
use crate::fetch::{self, BodyStream, FetchError};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream::{self, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;

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
}

#[uniffi::export(with_foreign)]
#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, MediaError>;
}

pub struct ForeignFetcher(pub Arc<dyn Fetcher>);

#[async_trait]
impl fetch::Fetcher for ForeignFetcher {
    async fn fetch(&self, request: fetch::Request) -> Result<fetch::Response, FetchError> {
        let (range_start, range_end) = request.range.map_or((None, None), |(start, end)| (Some(start), end));
        let request = FetchRequest { url: request.url, headers: request.headers, range_start, range_end };
        let response = self.0.fetch(request).await.map_err(network)?;
        Ok(fetch::Response { status: response.status, headers: response.headers, body: body_stream(response.body) })
    }
}

fn network(error: MediaError) -> FetchError {
    FetchError::Network(error.to_string())
}

fn body_stream(reader: Arc<dyn BodyReader>) -> BodyStream {
    stream::unfold(Some(reader), |state| async move {
        let reader = state?;
        match reader.next_chunk().await {
            Ok(Some(chunk)) => Some((Ok(Bytes::from(chunk)), Some(reader))),
            Ok(None) => None,
            Err(error) => Some((Err(network(error)), None)),
        }
    })
    .boxed()
}
