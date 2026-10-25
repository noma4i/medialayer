use super::load::{load_with, ImageLoadRequest};
use crate::api::MediaError;
use crate::fetch::mock::{MockFetcher, Script};
use crate::memory::InputLimits;
use crate::proxy::ImageOrigin;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

fn request(dir: &tempfile::TempDir, url: &str, cache_bytes: u64, timeout_ms: u64) -> ImageLoadRequest {
    ImageLoadRequest { url: url.to_string(), headers: HashMap::new(), dir: dir.path().display().to_string(), cache_bytes, timeout_ms }
}

fn fetcher_with(routes: &[(&str, Script)]) -> Arc<MockFetcher> {
    let fetcher = Arc::new(MockFetcher::new());
    for (url, script) in routes {
        fetcher.route(url, script.clone());
    }
    fetcher
}

#[test]
fn inv_image_56_second_load_of_a_url_comes_from_the_cache_without_network() {
    let dir = tempfile::tempdir().unwrap();
    let fetcher = fetcher_with(&[("https://img/a.png", Script::ok(vec![7u8; 300]))]);
    let load = || load_with(request(&dir, "https://img/a.png", 10_000, 2_000), fetcher.clone(), InputLimits::default()).unwrap();
    let first = load();
    let second = load();
    assert_eq!((first.source, second.source), (ImageOrigin::Network, ImageOrigin::Cache));
    assert_eq!(second.bytes, vec![7u8; 300]);
    assert_eq!(fetcher.calls(), 1);
}

#[test]
fn inv_image_57_cache_over_the_byte_limit_evicts_the_least_recently_used_entry() {
    let dir = tempfile::tempdir().unwrap();
    let fetcher = fetcher_with(&[
        ("https://img/a.png", Script::ok(vec![1u8; 600])),
        ("https://img/b.png", Script::ok(vec![2u8; 600])),
    ]);
    let load = |url: &str| load_with(request(&dir, url, 1_000, 2_000), fetcher.clone(), InputLimits::default()).unwrap();
    load("https://img/a.png");
    load("https://img/b.png");
    assert_eq!(load("https://img/b.png").source, ImageOrigin::Cache);
    assert_eq!(load("https://img/a.png").source, ImageOrigin::Network);
    assert_eq!((fetcher.calls_for("https://img/a.png"), fetcher.calls_for("https://img/b.png")), (2, 1));
}

#[test]
fn inv_image_58_no_answer_within_the_timeout_is_an_explicit_timeout_error() {
    let dir = tempfile::tempdir().unwrap();
    let fetcher = fetcher_with(&[("https://img/slow.png", Script::ok(vec![1u8; 10]).head_delay(Duration::from_secs(30)))]);
    let started = std::time::Instant::now();
    let error = load_with(request(&dir, "https://img/slow.png", 10_000, 100), fetcher, InputLimits::default()).unwrap_err();
    assert!(matches!(error, MediaError::Timeout { .. }), "{error:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn inv_image_59_response_above_the_input_limit_is_refused_and_not_cached() {
    let dir = tempfile::tempdir().unwrap();
    let fetcher = fetcher_with(&[("https://img/big.png", Script::ok(vec![9u8; 4_000]).chunked(1_000))]);
    let limits = InputLimits { response_bytes: 2_000, ..InputLimits::default() };
    let load = || load_with(request(&dir, "https://img/big.png", 100_000, 2_000), fetcher.clone(), limits);
    assert!(matches!(load(), Err(MediaError::Fetch { .. })));
    assert!(matches!(load(), Err(MediaError::Fetch { .. })));
    assert_eq!(fetcher.calls(), 2);
}

#[test]
fn inv_image_60_non_success_status_is_an_error_and_not_cached() {
    let dir = tempfile::tempdir().unwrap();
    let fetcher = fetcher_with(&[("https://img/gone.png", Script::ok(b"gone".to_vec()).status(404))]);
    let load = || load_with(request(&dir, "https://img/gone.png", 10_000, 2_000), fetcher.clone(), InputLimits::default());
    assert!(matches!(load(), Err(MediaError::Fetch { .. })));
    assert!(matches!(load(), Err(MediaError::Fetch { .. })));
    assert_eq!(fetcher.calls(), 2);
}
