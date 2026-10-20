use super::memory_store::MemoryStore;
use super::*;
use crate::fetch::mock::{MockFetcher, Script};
use crate::limits::Limits;
use std::time::Duration;

async fn rig(max_concurrent: usize) -> (Proxy, Arc<MockFetcher>) {
    let config = Config { max_concurrent, playback_reserved: 0, ..Config::default() };
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
    while fetcher.calls() == 0 {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_secs(2)).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }

    assert!(load.is_finished(), "the old timeout is still in force");
    assert!(matches!(load.await.unwrap(), Err(ProxyError::Fetch(_))));
}

#[tokio::test]
async fn inv_proxy_10_a_new_concurrency_limit_lets_waiting_downloads_start_in_place() {
    let (proxy, fetcher) = rig(1).await;
    for name in ["a", "b"] {
        fetcher.route(&format!("https://o.test/{name}.mp4"), Script::ok(b"x".to_vec()).hanging());
    }
    for name in ["a", "b"] {
        let proxy = proxy.clone();
        tokio::spawn(async move { proxy.fetch_into_store(&format!("https://o.test/{name}.mp4"), &HashMap::new(), Priority::Prefetch, None).await });
    }
    while fetcher.calls() < 1 {
        tokio::task::yield_now().await;
    }
    for _ in 0..1_000 {
        tokio::task::yield_now().await;
    }
    assert_eq!(fetcher.calls(), 1);

    proxy.set_limits(&Limits { max_concurrent: 2, ..Limits::default() });

    for _ in 0..100_000 {
        if fetcher.calls() == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(fetcher.calls(), 2);
}
