use super::test_client::{get, request};
use super::*;
use crate::fetch::mock::Script;
use std::time::Duration;
use super::proxy_harness::*;

#[tokio::test]
async fn inv_proxy_05_overlapping_ranges_fetch_only_the_missing_blocks() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(4500, 1);
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(data.clone()).chunked(300));
    let target = path_of(&rig.proxy.url_for("https://o.test/r.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();

    let first = request(port, "GET", &target, &[("Range", "bytes=1500-2499")]).await;
    assert_eq!((first.status, first.header("content-range")), (206, Some("bytes 1500-2499/4500")));
    assert_eq!(first.body, data[1500..2500]);
    assert_eq!(fetched_ranges(&rig), vec![(0, Some(0)), (1000, Some(1999)), (2000, Some(2999))]);
    let after_first = rig.fetcher.bytes_served();
    assert_eq!(after_first, 2000);

    let second = request(port, "GET", &target, &[("Range", "bytes=2000-3499")]).await;
    assert_eq!(second.body, data[2000..3500]);
    assert_eq!(fetched_ranges(&rig).last(), Some(&(3000, Some(3999))));
    assert_eq!(rig.fetcher.bytes_served() - after_first, 1000);

    let again = request(port, "GET", &target, &[("Range", "bytes=1500-3499")]).await;
    assert_eq!(again.body, data[1500..3500]);
    assert_eq!(rig.fetcher.bytes_served() - after_first, 1000);
    assert_eq!(rig.store.bytes("https://o.test/r.mp4#2"), Some(data[2000..3000].to_vec()));
    assert_eq!(rig.store.meta("https://o.test/r.mp4#1").await.unwrap().total_length, Some(4500));
}

#[tokio::test]
async fn inv_proxy_11_suffix_and_tail_ranges_use_a_short_last_block() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(4500, 3);
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(data.clone()));
    let target = path_of(&rig.proxy.url_for("https://o.test/r.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();
    let suffix = request(port, "GET", &target, &[("Range", "bytes=-300")]).await;
    assert_eq!((suffix.status, suffix.header("content-range")), (206, Some("bytes 4200-4499/4500")));
    assert_eq!(suffix.body, data[4200..]);
    assert_eq!(fetched_ranges(&rig).last(), Some(&(4000, Some(4499))));
    let beyond = request(port, "GET", &target, &[("Range", "bytes=9999-")]).await;
    assert_eq!((beyond.status, beyond.header("content-range")), (416, Some("bytes */4500")));
}

#[tokio::test]
async fn inv_proxy_05_parallel_ranges_over_one_block_fetch_it_once() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(3000, 4);
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(data.clone()).chunked(100).chunk_delay(Duration::from_millis(5)));
    let target = path_of(&rig.proxy.url_for("https://o.test/r.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();
    let clients: Vec<_> = (0..4)
        .map(|_| tokio::spawn({ let target = target.clone(); async move { request(port, "GET", &target, &[("Range", "bytes=1100-1900")]).await } }))
        .collect();
    for client in clients {
        assert_eq!(within(10, client).await.unwrap().body, data[1100..1901]);
    }
    assert_eq!(fetched_ranges(&rig).iter().filter(|range| range.0 == 1000).count(), 1);
}

#[tokio::test]
async fn inv_proxy_09_prefetch_over_the_wait_queue_limit_is_refused_at_once() {
    let mut config = config(2);
    config.max_waiting = 1;
    let rig = rig_with(config).await;
    for name in ["a", "b", "c"] {
        rig.fetcher.route(&format!("https://o.test/{name}.mp4"), Script::ok(pattern(10, 0)).hanging());
    }
    let running = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/a.mp4", &HashMap::new(), Priority::Prefetch, None).await } });
    assert!(eventually(|| rig.fetcher.calls_for("https://o.test/a.mp4") == 1).await);
    let waiting = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/b.mp4", &HashMap::new(), Priority::Prefetch, None).await } });
    within(5, async { while rig.proxy.inner.slots.waiting() != 1 { tokio::task::yield_now().await; } }).await;

    let refused = within(1, rig.proxy.fetch_into_store("https://o.test/c.mp4", &HashMap::new(), Priority::Prefetch, None)).await;
    assert_eq!(refused, Err(ProxyError::QueueFull));
    assert_eq!(rig.fetcher.calls_for("https://o.test/c.mp4"), 0);
    running.abort();
    waiting.abort();
}

#[tokio::test]
async fn inv_proxy_09_a_player_request_promotes_a_waiting_prefetch() {
    let rig = rig_with(config(2)).await;
    rig.fetcher.route("https://o.test/hold.mp4", Script::ok(pattern(10, 0)).hanging());
    rig.fetcher.route("https://o.test/next.mp4", Script::ok(pattern(10, 6)));
    let hold = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/hold.mp4", &HashMap::new(), Priority::Prefetch, None).await } });
    assert!(eventually(|| rig.fetcher.calls_for("https://o.test/hold.mp4") == 1).await);
    let queued = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/next.mp4", &HashMap::new(), Priority::Prefetch, None).await } });
    within(5, async { while rig.proxy.inner.slots.waiting() != 1 { tokio::task::yield_now().await; } }).await;
    assert_eq!(rig.fetcher.calls_for("https://o.test/next.mp4"), 0);

    let target = path_of(&rig.proxy.url_for("https://o.test/next.mp4", &HashMap::new()).unwrap());
    let reply = within(2, get(rig.proxy.port(), &target)).await;
    assert_eq!(reply.body, pattern(10, 6));
    assert_eq!(rig.fetcher.calls_for("https://o.test/next.mp4"), 1);
    assert_eq!(within(2, queued).await.unwrap(), Ok(super::ImageOrigin::Network));
    hold.abort();
}

#[tokio::test]
async fn inv_proxy_10_a_hung_origin_frees_its_slot_after_the_timeout() {
    let mut config = config(2);
    config.policy = config.policy.with_timeout(Duration::from_millis(300));
    let rig = rig_with(config).await;
    rig.fetcher.route("https://o.test/hang.mp4", Script::ok(pattern(10, 0)).hanging());
    rig.fetcher.route("https://o.test/after.mp4", Script::ok(pattern(10, 2)));
    let hung = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/hang.mp4", &HashMap::new(), Priority::Prefetch, None).await } });
    assert!(eventually(|| rig.fetcher.calls_for("https://o.test/hang.mp4") == 1).await);
    let queued = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/after.mp4", &HashMap::new(), Priority::Prefetch, None).await } });

    assert!(matches!(within(2, hung).await.unwrap(), Err(ProxyError::Fetch(_))));
    assert_eq!(within(2, queued).await.unwrap(), Ok(super::ImageOrigin::Network));
    assert_eq!(rig.store.bytes("https://o.test/after.mp4"), Some(pattern(10, 2)));
}

#[tokio::test]
async fn inv_proxy_03_a_prefetched_block_loads_alone_and_serves_later_ranges_without_the_origin() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(4500, 7);
    let url = "https://o.test/blocks.mp4";
    rig.fetcher.route(url, Script::ok(data.clone()).chunked(300));
    let none = HashMap::new();

    rig.proxy.fetch_first_block(url, &none, Priority::Prefetch).await.unwrap();
    assert_eq!(fetched_ranges(&rig), vec![(0, Some(999))]);
    assert_eq!(rig.fetcher.bytes_served(), 1000);
    assert_eq!(rig.store.bytes("https://o.test/blocks.mp4#0"), Some(data[..1000].to_vec()));
    assert_eq!(rig.store.meta("https://o.test/blocks.mp4#0").await.unwrap().total_length, Some(4500));

    rig.proxy.fetch_first_block(url, &none, Priority::Prefetch).await.unwrap();
    rig.proxy.fetch_block_into_store(url, 2, &none, Priority::Prefetch).await.unwrap();
    assert_eq!(fetched_ranges(&rig), vec![(0, Some(999)), (2000, Some(2999))]);

    let calls = rig.fetcher.calls();
    let target = path_of(&rig.proxy.url_for(url, &none).unwrap());
    let ranged = request(rig.proxy.port(), "GET", &target, &[("Range", "bytes=2100-2199")]).await;
    assert_eq!(ranged.body, data[2100..2200]);
    assert_eq!(rig.fetcher.calls(), calls);
}

#[tokio::test]
async fn inv_proxy_03_a_small_range_from_zero_is_served_from_the_prefetched_first_block() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(4500, 7);
    let url = "https://o.test/first.mp4";
    rig.fetcher.route(url, Script::ok(data.clone()).chunked(300));
    let none = HashMap::new();
    rig.proxy.fetch_first_block(url, &none, Priority::Prefetch).await.unwrap();
    let calls = rig.fetcher.calls();

    let target = path_of(&rig.proxy.url_for(url, &none).unwrap());
    let reply = request(rig.proxy.port(), "GET", &target, &[("Range", "bytes=0-99")]).await;

    assert_eq!((reply.status, reply.header("content-range")), (206, Some("bytes 0-99/4500")));
    assert_eq!(reply.body, data[..100]);
    assert_eq!(rig.fetcher.calls(), calls);
}
