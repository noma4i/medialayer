use super::*;
use crate::fetch::mock::{MockFetcher, Script};
use crate::proxy::{memory_store::MemoryStore, Config, Proxy};

async fn permits_returned(proxy: &Proxy) {
    for _ in 0..400 {
        if proxy.inner.slots.in_use() == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert_eq!(proxy.inner.slots.in_use(), 0, "a cancelled probe kept its permit");
}

#[tokio::test]
async fn inv_proxy_28_cancelled_probes_release_the_last_lock_in_either_order() {
    let fetcher = Arc::new(MockFetcher::new());
    let proxy = Proxy::start(Config::default(), fetcher.clone(), Arc::new(MemoryStore::new())).await.unwrap();
    let headers = HashMap::new();
    for owner_first in [false, true] {
        for index in 0..20 {
            let url = format!("https://o.test/{owner_first}/{index}.mp4");
            fetcher.route(&url, Script::ok(vec![1; 8]).hanging());
            let mut owner = Box::pin(known_total(&proxy.inner, &url, &url, &headers, None));
            assert!(futures_util::poll!(owner.as_mut()).is_pending());
            assert_eq!(fetcher.calls_for(&url), 1);
            let mut waiter = Box::pin(known_total(&proxy.inner, &url, &url, &headers, None));
            assert!(futures_util::poll!(waiter.as_mut()).is_pending());
            assert_eq!(fetcher.calls_for(&url), 1);
            assert_eq!(proxy.inner.probe_count(), 1);
            if owner_first {
                drop(owner);
                assert_eq!(proxy.inner.probe_count(), 1);
                assert!(futures_util::poll!(waiter.as_mut()).is_pending());
                assert_eq!(fetcher.calls_for(&url), 2);
                drop(waiter);
            } else {
                drop(waiter);
                assert_eq!(proxy.inner.probe_count(), 1);
                drop(owner);
            }
            assert_eq!(proxy.inner.probe_count(), 0, "cancelled probe left its table entry");
            permits_returned(&proxy).await;
        }
    }
    for (index, script) in [Script::ok(vec![1; 8]), Script::ok(vec![1; 8]).status(500)].into_iter().enumerate() {
        let url = format!("https://o.test/finished/{index}.mp4");
        fetcher.route(&url, script);
        let result = known_total(&proxy.inner, &url, &url, &headers, None).await;
        assert_eq!(result.is_ok(), index == 0);
        assert_eq!(proxy.inner.probe_count(), 0);
        permits_returned(&proxy).await;
    }
}
