use super::memory_store::{Failure, MemoryStore};
use super::test_client::{get, open, read_at_least, request};
use super::*;
use crate::fetch::mock::{MockFetcher, Script};
use std::time::Duration;
use super::proxy_harness::*;

#[tokio::test]
async fn inv_proxy_12_a_master_playlist_without_endlist_survives_a_restart_and_plays_offline() {
    let store = MemoryStore::new();
    let master = "https://o.test/live/master.m3u8";
    let first_fetcher = Arc::new(MockFetcher::new());
    first_fetcher.route(master, Script::ok(b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\nhttps://o.test/live/media.m3u8\n".to_vec()));
    let mut short_ttl = config(8);
    short_ttl.playlist_ttl = Duration::from_millis(50);
    let first = Proxy::start(short_ttl.clone(), first_fetcher.clone(), Arc::new(store.clone())).await.unwrap();
    let online = get(first.port(), &path_of(&first.url_for(master, &HashMap::new()).unwrap())).await;
    first.stop().await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    let offline = Arc::new(MockFetcher::new());
    offline.set_offline(true);
    let second = Proxy::start(short_ttl, offline.clone(), Arc::new(store)).await.unwrap();
    let replay = get(second.port(), &path_of(&second.url_for(master, &HashMap::new()).unwrap())).await;
    assert_eq!((replay.status, replay.body), (200, online.body));
    assert_eq!(offline.calls(), 0);
}

#[tokio::test]
async fn inv_proxy_02_a_store_failure_at_any_step_never_cuts_playback_for_any_joined_client() {
    for failure in [Failure::Begin, Failure::Append(1), Failure::Append(4), Failure::Commit] {
        let rig = rig().await;
        rig.store.fail_at(Some(failure));
        rig.fetcher.route(
            "https://o.test/big.mp4",
            Script::ok(pattern(5000, 9)).chunked(500).chunk_delay(Duration::from_millis(5)),
        );
        let target = path_of(&rig.proxy.url_for("https://o.test/big.mp4", &HashMap::new()).unwrap());
        let port = rig.proxy.port();
        let clients: Vec<_> = (0..3)
            .map(|_| tokio::spawn({ let target = target.clone(); async move { get(port, &target).await } }))
            .collect();
        for client in clients {
            let reply = within(10, client).await.unwrap();
            assert_eq!((reply.status, reply.body.len()), (200, 5000), "{failure:?}");
            assert_eq!(reply.body, pattern(5000, 9), "{failure:?}");
        }
        assert!(eventually(|| rig.store.open_writers() == 0 && rig.store.count() == 0).await, "{failure:?}");
        assert_eq!(rig.proxy.live_tasks(), 0, "{failure:?}");
    }
}

#[tokio::test]
async fn inv_proxy_02_a_store_failure_inside_a_block_never_cuts_a_range() {
    for failure in [Failure::Begin, Failure::Append(2), Failure::Commit] {
        let rig = rig_with(block_config(1000)).await;
        rig.store.fail_at(Some(failure));
        let data = pattern(4500, 5);
        rig.fetcher.route("https://o.test/r.mp4", Script::ok(data.clone()).chunked(250));
        let target = path_of(&rig.proxy.url_for("https://o.test/r.mp4", &HashMap::new()).unwrap());
        let reply = request(rig.proxy.port(), "GET", &target, &[("Range", "bytes=1500-3499")]).await;
        assert_eq!((reply.status, reply.header("content-range")), (206, Some("bytes 1500-3499/4500")), "{failure:?}");
        assert_eq!(reply.body, data[1500..3500], "{failure:?}");
    }
}

#[tokio::test]
async fn inv_proxy_01_registered_headers_reach_the_streams_own_urls_and_no_other_url_of_the_host() {
    let rig = rig().await;
    hls_origin(&rig);
    rig.fetcher.route("https://o.test/hls/other.mp4", Script::ok(pattern(10, 0)));
    let token = |value: &str| HashMap::from([("Authorization".to_string(), value.to_string())]);
    let media = rig.proxy.url_for(MEDIA, &token("token-stream")).unwrap();
    let other = rig.proxy.url_for("https://o.test/hls/other.mp4", &token("token-other")).unwrap();
    let port = rig.proxy.port();

    let playlist = get(port, &path_of(&media)).await;
    for line in media_lines(&playlist) {
        get(port, &follow(&media, &line)).await;
    }
    get(port, &path_of(&other)).await;

    let seen = |url: &str| {
        let request = rig.fetcher.requests().into_iter().find(|request| request.url == url).unwrap();
        request.headers.get("Authorization").cloned()
    };
    assert_eq!(seen(SEGMENT_ABSOLUTE).as_deref(), Some("token-stream"));
    assert_eq!(seen(SEGMENT_RELATIVE).as_deref(), Some("token-stream"));
    assert_eq!(seen("https://o.test/hls/other.mp4").as_deref(), Some("token-other"));
}

#[tokio::test]
async fn inv_proxy_06_abandoning_a_block_prefetch_cancels_its_download() {
    let rig = rig_with(block_config(1000)).await;
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(pattern(4500, 1)).chunked(50).chunk_delay(Duration::from_millis(10)));
    let owner = tokio::spawn({
        let proxy = rig.proxy.clone();
        async move { proxy.fetch_first_block("https://o.test/r.mp4", &HashMap::new(), Priority::Prefetch).await }
    });
    assert!(eventually(|| rig.store.open_writers() == 1).await);
    owner.abort();
    assert!(eventually(|| rig.proxy.live_tasks() == 0).await);
    assert_eq!((rig.store.bytes("https://o.test/r.mp4#0"), rig.store.commits()), (None, 0));
}

#[tokio::test]
async fn inv_proxy_43_a_block_cut_by_the_origin_keeps_its_bytes_and_the_next_attempt_asks_only_for_the_rest() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(4500, 3);
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(data.clone()).chunked(100).cut_after(400));

    let mut attempts = Vec::new();
    for _ in 0..3 {
        attempts.push(rig.proxy.fetch_first_block("https://o.test/r.mp4", &HashMap::new(), Priority::Prefetch).await.is_ok());
    }

    let ranges: Vec<_> = rig.fetcher.requests().into_iter().map(|request| request.range).collect();
    assert_eq!(attempts, vec![false, false, true], "{ranges:?}");
    assert_eq!(ranges, vec![Some((0, Some(999))), Some((400, Some(999))), Some((800, Some(999)))]);
    assert_eq!(rig.store.bytes("https://o.test/r.mp4#0"), Some(data[..1000].to_vec()));
}

#[tokio::test]
async fn inv_proxy_43_purging_the_source_drops_its_kept_bytes() {
    let rig = rig_with(block_config(1000)).await;
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(pattern(4500, 4)).chunked(100).cut_after(100));
    let _ = rig.proxy.fetch_block_into_store("https://o.test/r.mp4", 1, &HashMap::new(), Priority::Prefetch).await;
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(pattern(4500, 4)).chunked(100));

    rig.proxy.forget_downloads(Some("https://o.test/r.mp4"));
    rig.proxy.fetch_block_into_store("https://o.test/r.mp4", 1, &HashMap::new(), Priority::Prefetch).await.unwrap();

    let last = rig.fetcher.requests().last().map(|request| request.range);
    assert_eq!(last, Some(Some((1000, Some(1999)))));
}

#[tokio::test]
async fn inv_proxy_06_a_range_dropped_mid_block_lets_the_block_finish_for_the_next_range() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(4500, 2);
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(data.clone()).chunked(100).chunk_delay(Duration::from_millis(10)));
    let target = path_of(&rig.proxy.url_for("https://o.test/r.mp4", &HashMap::new()).unwrap());
    let mut stream = open(rig.proxy.port(), "GET", &target, &[("Range", "bytes=1100-1299")]).await;
    read_at_least(&mut stream, 300).await;
    drop(stream);
    assert!(eventually(|| rig.store.bytes("https://o.test/r.mp4#1").is_some()).await);
    assert_eq!(rig.store.bytes("https://o.test/r.mp4#1"), Some(data[1000..2000].to_vec()));
    assert!(eventually(|| rig.proxy.live_tasks() == 0).await);
}

#[tokio::test]
async fn inv_proxy_07_dropping_the_last_proxy_closes_the_listener_connections_and_downloads() {
    let rig = rig().await;
    rig.fetcher.route(
        "https://o.test/slow.mp4",
        Script::filled(10 * 1024 * 1024).chunked(16 * 1024).chunk_delay(Duration::from_millis(5)),
    );
    let target = path_of(&rig.proxy.url_for("https://o.test/slow.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();
    let mut stream = open(port, "GET", &target, &[]).await;
    read_at_least(&mut stream, 32 * 1024).await;
    let Rig { proxy, store, .. } = rig;
    let clone = proxy.clone();
    drop(proxy);
    assert_eq!(store.open_writers(), 1);
    drop(clone);

    assert!(eventually(|| store.open_writers() == 0 && store.count() == 0).await);
    assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err());
}
