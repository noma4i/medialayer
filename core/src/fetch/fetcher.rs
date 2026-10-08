use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream::BoxStream;
use std::collections::HashMap;

pub type BodyStream = BoxStream<'static, Result<Bytes, FetchError>>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Request {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub range: Option<(u64, Option<u64>)>,
}

pub struct Response {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: BodyStream,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    #[error("network error: {0}")]
    Network(String),
    #[error("timeout")]
    Timeout,
    #[error("status {0}")]
    Status(u16),
}

#[async_trait]
pub trait Fetcher: Send + Sync {
    async fn fetch(&self, request: Request) -> Result<Response, FetchError>;
}

pub fn header_value<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
}
