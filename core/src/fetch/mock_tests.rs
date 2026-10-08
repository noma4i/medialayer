use super::mock::{MockFetcher, Script};
use super::*;
use futures_util::StreamExt;
use std::time::Duration;

fn request(url: &str) -> Request {
    Request { url: url.to_string(), ..Request::default() }
}

async fn collect(response: Response) -> Result<Vec<u8>, FetchError> {
    let mut body = response.body;
    let mut out = Vec::new();
    while let Some(chunk) = body.next().await {
        out.extend_from_slice(&chunk?);
    }
    Ok(out)
}

#[tokio::test]
async fn serves_a_scripted_body_and_counts_calls() {
    let mock = MockFetcher::new();
    mock.route("http://o/a", Script::ok(b"hello".to_vec()));
    let response = mock.fetch(request("http://o/a")).await.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(header_value(&response.headers, "content-length"), Some("5"));
    assert_eq!(collect(response).await.unwrap(), b"hello");
    assert_eq!(mock.calls(), 1);
    assert_eq!(mock.calls_for("http://o/a"), 1);
    assert_eq!(mock.calls_for("http://o/b"), 0);
}

#[tokio::test]
async fn unknown_url_is_404() {
    let mock = MockFetcher::new();
    let response = mock.fetch(request("http://o/none")).await.unwrap();
    assert_eq!(response.status, 404);
}

#[tokio::test]
async fn honors_ranges_with_206_and_content_range() {
    let mock = MockFetcher::new();
    mock.route("http://o/a", Script::ok((0u8..100).collect::<Vec<_>>()));
    let mut req = request("http://o/a");
    req.range = Some((10, Some(19)));
    let response = mock.fetch(req).await.unwrap();
    assert_eq!(response.status, 206);
    assert_eq!(header_value(&response.headers, "content-range"), Some("bytes 10-19/100"));
    assert_eq!(collect(response).await.unwrap(), (10u8..20).collect::<Vec<_>>());

    let mut open = request("http://o/a");
    open.range = Some((95, None));
    let tail = mock.fetch(open).await.unwrap();
    assert_eq!(collect(tail).await.unwrap(), (95u8..100).collect::<Vec<_>>());
}

#[tokio::test]
async fn records_requests_with_headers() {
    let mock = MockFetcher::new();
    let mut req = request("http://o/a");
    req.headers.insert("Authorization".to_string(), "t".to_string());
    mock.fetch(req).await.unwrap();
    let recorded = mock.requests();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].headers.get("Authorization").map(String::as_str), Some("t"));
}

#[tokio::test]
async fn offline_fails_every_call_with_a_network_error() {
    let mock = MockFetcher::new();
    mock.route("http://o/a", Script::ok(b"x".to_vec()));
    mock.set_offline(true);
    let error = mock.fetch(request("http://o/a")).await.err().unwrap();
    assert!(matches!(error, FetchError::Network(_)));
    assert_eq!(mock.calls(), 1);
}

#[tokio::test]
async fn cut_after_breaks_the_body_midway() {
    let mock = MockFetcher::new();
    mock.route("http://o/a", Script::ok(vec![1u8; 100]).chunked(10).cut_after(30));
    let response = mock.fetch(request("http://o/a")).await.unwrap();
    assert!(matches!(collect(response).await, Err(FetchError::Network(_))));
}

#[tokio::test(start_paused = true)]
async fn head_delay_postpones_the_response() {
    let mock = MockFetcher::new();
    mock.route("http://o/a", Script::ok(b"x".to_vec()).head_delay(Duration::from_secs(5)));
    let started = tokio::time::Instant::now();
    mock.fetch(request("http://o/a")).await.unwrap();
    assert!(started.elapsed() >= Duration::from_secs(5));
}

#[tokio::test(start_paused = true)]
async fn hanging_script_never_answers() {
    let mock = MockFetcher::new();
    mock.route("http://o/a", Script::ok(b"x".to_vec()).hanging());
    let outcome = tokio::time::timeout(Duration::from_secs(60), mock.fetch(request("http://o/a"))).await;
    assert!(outcome.is_err());
}

#[tokio::test]
async fn tracks_live_chunks_for_a_filled_payload() {
    let mock = MockFetcher::new();
    mock.route("http://o/big", Script::filled(1024 * 1024).chunked(64 * 1024));
    let response = mock.fetch(request("http://o/big")).await.unwrap();
    let mut body = response.body;
    let mut total = 0;
    while let Some(chunk) = body.next().await {
        total += chunk.unwrap().len();
    }
    assert_eq!(total, 1024 * 1024);
    assert_eq!(mock.live_chunks(), 0);
    assert!(mock.peak_chunks() <= 2);
}
