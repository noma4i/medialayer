use super::memory_store::MemoryStore;
use super::*;
use crate::fetch::mock::{MockFetcher, Script};
use crate::limits::Limits;
use super::proxy_harness::eventually;
use std::time::Duration;

async fn rig(max_concurrent: usize) -> (Proxy, Arc<MockFetcher>) {
    let config = Config { max_concurrent, playback_reserved: 0, header_read_timeout: None, ..Config::default() };
    let fetcher = Arc::new(MockFetcher::new());
    let proxy = Proxy::start(config, fetcher.clone(), Arc::new(MemoryStore::new())).await.unwrap();
    (proxy, fetcher)
}

#[tokio::test(start_paused = true)]
async fn inv_proxy_10_a_new_timeout_applies_to_the_next_origin_wait_without_a_restart() {
    let (proxy, fetcher) = rig(2).await;
    fetcher.route("https://o.test/hang.mp4", Script::ok(b"x".to_vec()).hanging());

    proxy.set_limits(&Limits { timeout_seconds: 1, ..Limits::default() });
    let load = tokio::spawn({
        let proxy = proxy.clone();
        async move { proxy.fetch_into_store("https://o.test/hang.mp4", &HashMap::new(), Priority::Prefetch, None).await }
    });
    eventually(|| fetcher.calls() > 0).await;
    tokio::time::advance(Duration::from_secs(2)).await;
    let result = tokio::time::timeout(Duration::from_secs(1), load).await.expect("the old timeout is still in force");
    assert!(matches!(result.unwrap(), Err(ProxyError::Fetch(_))));
}

#[tokio::test]
async fn inv_proxy_10_a_new_concurrency_limit_lets_waiting_downloads_start_in_place() {
    let (proxy, fetcher) = rig(1).await;
    for name in ["a", "b"] {
        fetcher.route(&format!("https://o.test/{name}.mp4"), Script::ok(b"x".to_vec()).hanging());
    }
    let first = proxy.clone();
    tokio::spawn(async move { first.fetch_into_store("https://o.test/a.mp4", &HashMap::new(), Priority::Prefetch, None).await });
    eventually(|| fetcher.calls() == 1).await;
    let other = proxy.clone();
    let second = tokio::spawn(async move {
        other.fetch_into_store("https://o.test/b.mp4", &HashMap::new(), Priority::Prefetch, None).await
    });
    eventually(|| proxy.inner.slots.waiting() == 1).await;
    assert_eq!(fetcher.calls(), 1);

    proxy.set_limits(&Limits { max_concurrent: 2, ..Limits::default() });

    eventually(|| fetcher.calls() == 2).await;
    assert_eq!(fetcher.calls(), 2);
    second.abort();
}
