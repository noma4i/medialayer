use super::*;
use crate::api::MediaError;
use crate::fetch::{self, FetchError, Fetcher as _};
use crate::memory::InputLimits;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

struct Chunks(Mutex<Vec<Result<Option<Vec<u8>>, MediaError>>>, Arc<std::sync::atomic::AtomicUsize>);

#[async_trait]
impl BodyReader for Chunks {
    async fn next_chunk(&self) -> Result<Option<Vec<u8>>, MediaError> {
        self.0.lock().unwrap().remove(0)
    }

    fn release(&self) {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

struct Platform {
    seen: Mutex<Vec<FetchRequest>>,
    chunks: Vec<Result<Option<Vec<u8>>, MediaError>>,
    closes: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl Fetcher for Platform {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, MediaError> {
        self.seen.lock().unwrap().push(request);
        let body = Arc::new(Chunks(Mutex::new(self.chunks.clone()), self.closes.clone()));
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
async fn inv_ffi_01_request_and_range_cross_to_the_platform_and_the_body_streams_back() {
    let platform = Arc::new(Platform {
        seen: Mutex::new(Vec::new()),
        chunks: vec![Ok(Some(b"ab".to_vec())), Ok(Some(b"cd".to_vec())), Ok(None)],
        closes: Arc::default(),
    });
    let response = ForeignFetcher::new(platform.clone(), InputLimits::default()).fetch(request()).await.unwrap();

    let body: Vec<Result<Bytes, FetchError>> = response.body.collect().await;

    assert_eq!((response.status, response.headers["etag"].as_str()), (206, "v1"));
    assert_eq!(body, vec![Ok(Bytes::from("ab")), Ok(Bytes::from("cd"))]);
    let seen = platform.seen.lock().unwrap();
    assert_eq!((seen[0].range_start, seen[0].range_end, seen[0].headers["x"].as_str()), (Some(10), Some(19), "1"));
}

#[tokio::test]
async fn inv_ffi_01_a_chunk_over_the_response_limit_ends_the_body_with_an_error_and_is_not_passed_on() {
    let platform = Arc::new(Platform { seen: Mutex::new(Vec::new()), chunks: vec![Ok(Some(b"ab".to_vec())), Ok(Some(b"cdef".to_vec())), Ok(None)], closes: Arc::default() });
    let limits = InputLimits { response_bytes: 3, ..InputLimits::default() };
    let response = ForeignFetcher::new(platform, limits).fetch(request()).await.unwrap();

    let body: Vec<Result<Bytes, FetchError>> = response.body.collect().await;

    assert_eq!(body.len(), 2);
    assert_eq!(body[0], Ok(Bytes::from("ab")));
    assert!(matches!(&body[1], Err(FetchError::Network(detail)) if detail.contains("limit")));
}

#[tokio::test]
async fn inv_ffi_01_a_platform_error_in_the_middle_of_the_body_ends_the_stream_with_a_fetch_error() {
    let failure = MediaError::Fetch { detail: "connection reset".to_string() };
    let platform = Arc::new(Platform { seen: Mutex::new(Vec::new()), chunks: vec![Ok(Some(b"ab".to_vec())), Err(failure)], closes: Arc::default() });
    let response = ForeignFetcher::new(platform, InputLimits::default()).fetch(request()).await.unwrap();

    let body: Vec<Result<Bytes, FetchError>> = response.body.collect().await;

    assert_eq!(body.len(), 2);
    assert!(matches!(&body[1], Err(FetchError::Network(detail)) if detail.contains("connection reset")));
}

#[tokio::test]
async fn inv_ffi_01_the_platform_reader_is_closed_when_the_body_is_dropped_midway_and_when_it_ends() {
    for read_to_end in [false, true] {
        let platform = Arc::new(Platform {
            seen: Mutex::new(Vec::new()),
            chunks: vec![Ok(Some(b"ab".to_vec())), Ok(Some(b"cd".to_vec())), Ok(None)],
            closes: Arc::default(),
        });
        let response = ForeignFetcher::new(platform.clone(), InputLimits::default()).fetch(request()).await.unwrap();
        let mut body = response.body;

        if read_to_end {
            while body.next().await.is_some() {}
        } else {
            body.next().await;
        }
        drop(body);

        for _ in 0..1_000_000 {
            if platform.closes.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(platform.closes.load(std::sync::atomic::Ordering::SeqCst), 1, "read_to_end={read_to_end}");
    }
}
