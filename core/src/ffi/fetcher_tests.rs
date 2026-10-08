use super::*;
use crate::api::MediaError;
use crate::fetch::{self, FetchError, Fetcher as _};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

struct Chunks(Mutex<Vec<Result<Option<Vec<u8>>, MediaError>>>);

#[async_trait]
impl BodyReader for Chunks {
    async fn next_chunk(&self) -> Result<Option<Vec<u8>>, MediaError> {
        self.0.lock().unwrap().remove(0)
    }
}

struct Platform {
    seen: Mutex<Vec<FetchRequest>>,
    chunks: Vec<Result<Option<Vec<u8>>, MediaError>>,
}

#[async_trait]
impl Fetcher for Platform {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, MediaError> {
        self.seen.lock().unwrap().push(request);
        let body = Arc::new(Chunks(Mutex::new(self.chunks.clone())));
        Ok(FetchResponse { status: 206, headers: HashMap::from([("etag".to_string(), "v1".to_string())]), body })
    }
}

fn request() -> fetch::Request {
    fetch::Request {
        url: "https://cdn.test/a.mp4".to_string(),
        headers: HashMap::from([("x".to_string(), "1".to_string())]),
        range: Some((10, Some(19))),
    }
}

#[tokio::test]
async fn request_and_range_cross_to_the_platform_and_the_body_streams_back() {
    let platform = Arc::new(Platform {
        seen: Mutex::new(Vec::new()),
        chunks: vec![Ok(Some(b"ab".to_vec())), Ok(Some(b"cd".to_vec())), Ok(None)],
    });
    let response = ForeignFetcher(platform.clone()).fetch(request()).await.unwrap();

    let body: Vec<Result<Bytes, FetchError>> = response.body.collect().await;

    assert_eq!((response.status, response.headers["etag"].as_str()), (206, "v1"));
    assert_eq!(body, vec![Ok(Bytes::from("ab")), Ok(Bytes::from("cd"))]);
    let seen = platform.seen.lock().unwrap();
    assert_eq!((seen[0].range_start, seen[0].range_end, seen[0].headers["x"].as_str()), (Some(10), Some(19), "1"));
}

#[tokio::test]
async fn a_platform_error_in_the_middle_of_the_body_ends_the_stream_with_a_fetch_error() {
    let failure = MediaError::Fetch { detail: "connection reset".to_string() };
    let platform = Arc::new(Platform { seen: Mutex::new(Vec::new()), chunks: vec![Ok(Some(b"ab".to_vec())), Err(failure)] });
    let response = ForeignFetcher(platform).fetch(request()).await.unwrap();

    let body: Vec<Result<Bytes, FetchError>> = response.body.collect().await;

    assert_eq!(body.len(), 2);
    assert!(matches!(&body[1], Err(FetchError::Network(detail)) if detail.contains("connection reset")));
}
