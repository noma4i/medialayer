use super::memory_store::MemoryStore;
use super::*;
use crate::fetch::mock::{MockFetcher, Script};

const URL: &str = "https://o.test/blocked.mp4";

#[tokio::test]
async fn inv_proxy_36_a_slow_rejection_body_stops_when_the_last_consumer_leaves() {
    let fetcher = Arc::new(MockFetcher::new());
    fetcher.route(URL, Script::ok(vec![1; 100]).status(403).chunk_delay(Duration::from_secs(3600)));
    let proxy = Proxy::start(Config::default(), fetcher.clone(), Arc::new(MemoryStore::new())).await.unwrap();

    let subscription = proxy.inner.get_or_start(URL, HashMap::new(), Priority::Playback);
    assert!(super::proxy_harness::eventually(|| fetcher.calls_for(URL) == 1).await);
    assert_eq!(fetcher.calls_for(URL), 1);
    drop(subscription);

    assert!(super::proxy_harness::eventually(|| proxy.live_tasks() == 0 && proxy.inner.slots.in_use() == 0).await);
}
