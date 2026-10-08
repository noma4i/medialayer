use crate::proxy::test_client::{request, Reply};
use super::test_support::*;
use super::*;
use crate::fetch::mock::{MockFetcher, Script};
use crate::limits::EvictionStrategy;
use crate::proxy::{Config, Proxy};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

const MASTER: &str = "https://o.test/hls/master.m3u8";
const MEDIA: &str = "https://o.test/hls/media.m3u8";
const SEGMENT_ABSOLUTE: &str = "https://cdn.test/seg0.ts";
const SEGMENT_RELATIVE: &str = "https://o.test/hls/seg1.ts";
const BLOB: &str = "https://o.test/blob";
const VIDEO: &str = "https://o.test/v.mp4";
const ROOMY: u64 = 10 * 1024 * 1024;
const SMALL_BUDGET: u64 = 4000;

struct Rig {
    proxy: Proxy,
    fetcher: Arc<MockFetcher>,
    cache: Cache,
    dir: tempfile::TempDir,
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|index| (index as u8).wrapping_add(seed)).collect()
}

async fn rig(budget: u64) -> Rig {
    let f = fixture(limits(budget, 0.9, EvictionStrategy::Lru), &[]).await;
    let fetcher = Arc::new(MockFetcher::new());
    let proxy = start(&f.cache, fetcher.clone()).await;
    Rig { proxy, fetcher, cache: f.cache, dir: f.dir }
}

async fn start(cache: &Cache, fetcher: Arc<MockFetcher>) -> Proxy {
    let config = Config { playlist_ttl: Duration::from_secs(60), ..Config::default() };
    let store = CacheStore::new(cache.clone(), Arc::new(classify_kind));
    Proxy::start(config, fetcher, Arc::new(store)).await.expect("proxy start")
}

async fn downloads_finished(proxy: &Proxy) {
    for _ in 0..5_000 {
        if proxy.live_tasks() == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("downloads did not finish");
}

async fn reopen_offline(rig: Rig) -> Rig {
    downloads_finished(&rig.proxy).await;
    rig.proxy.stop().await;
    rig.cache.flush().await;
    let cache = Cache::open_with(rig.dir.path(), limits(ROOMY, 0.9, EvictionStrategy::Lru), options(rig_clock(), &[]));
    cache.ready().await;
    let fetcher = Arc::new(MockFetcher::new());
    fetcher.set_offline(true);
    let proxy = start(&cache, fetcher.clone()).await;
    Rig { proxy, fetcher, cache, dir: rig.dir }
}

fn rig_clock() -> Arc<ManualClock> {
    Arc::new(ManualClock::new(1_000))
}

fn hls_origin(fetcher: &MockFetcher) {
    fetcher.route(MASTER, Script::ok(b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\nmedia.m3u8\n".to_vec()));
    fetcher.route(
        MEDIA,
        Script::ok(b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\nhttps://cdn.test/seg0.ts\n#EXTINF:2.0,\nseg1.ts\n#EXT-X-ENDLIST\n".to_vec()),
    );
    fetcher.route(SEGMENT_ABSOLUTE, Script::ok(pattern(3000, 1)).chunked(700));
    fetcher.route(SEGMENT_RELATIVE, Script::ok(pattern(2000, 9)).chunked(700));
}

async fn fetch(rig: &Rig, url: &str, range: Option<&str>) -> Reply {
    let proxied = rig.proxy.url_for(url, &HashMap::new()).expect("proxy url");
    let parsed = Url::parse(&proxied).unwrap();
    let target = format!("{}?{}", parsed.path(), parsed.query().unwrap_or_default());
    match range {
        Some(range) => request(rig.proxy.port(), "GET", &target, &[("Range", range)]).await,
        None => request(rig.proxy.port(), "GET", &target, &[]).await,
    }
}

async fn play(rig: &Rig, urls: &[&str]) -> Vec<Reply> {
    let mut replies = Vec::new();
    for url in urls {
        let reply = fetch(rig, url, None).await;
        assert_eq!(reply.status, 200, "{url}");
        replies.push(reply);
    }
    replies
}

async fn settle(cache: &Cache) {
    let target = SMALL_BUDGET * 9 / 10;
    for _ in 0..400 {
        if cache.stats().await.total_bytes <= target {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("eviction did not reach its target");
}

#[tokio::test]
async fn hls_and_media_replay_offline_from_a_reopened_disk_cache_with_their_content_type() {
    let first = rig(ROOMY).await;
    hls_origin(&first.fetcher);
    first.fetcher.route(BLOB, Script::ok(pattern(100, 5)).header("Content-Type", "video/x-custom"));
    let urls = [MASTER, MEDIA, SEGMENT_ABSOLUTE, SEGMENT_RELATIVE, BLOB];
    let online = play(&first, &urls).await;
    assert_eq!(online[2].body, pattern(3000, 1));
    assert_eq!(online[4].header("content-type"), Some("video/x-custom"));

    let second = reopen_offline(first).await;
    let replay = play(&second, &urls[1..]).await;

    assert_eq!(second.fetcher.calls(), 0);
    for (before, after) in online[1..].iter().zip(&replay) {
        assert_eq!(before.body, after.body);
    }
    assert_eq!(replay[3].header("content-type"), Some("video/x-custom"));
    assert_eq!(second.cache.stats().await.file_count, 5);
}

#[tokio::test]
async fn range_requests_are_cut_from_cached_blocks_and_their_total_after_a_reopen() {
    let first = rig(ROOMY).await;
    let data = pattern(100, 0);
    first.fetcher.route(VIDEO, Script::ok(data.clone()));
    let tail = fetch(&first, VIDEO, Some("bytes=50-")).await;
    assert_eq!((tail.status, tail.header("content-range")), (206, Some("bytes 50-99/100")));
    assert_eq!(tail.body, data[50..]);

    let second = reopen_offline(first).await;
    let middle = fetch(&second, VIDEO, Some("bytes=10-19")).await;
    let suffix = fetch(&second, VIDEO, Some("bytes=-10")).await;

    assert_eq!((middle.status, middle.header("content-range")), (206, Some("bytes 10-19/100")));
    assert_eq!(middle.body, data[10..20]);
    assert_eq!(suffix.body, data[90..]);
    assert_eq!(second.fetcher.calls(), 0);
}

#[tokio::test]
async fn a_small_budget_evicts_old_entries_without_breaking_what_is_served() {
    let rig = rig(SMALL_BUDGET).await;
    hls_origin(&rig.fetcher);
    let urls = [MASTER, MEDIA, SEGMENT_ABSOLUTE, SEGMENT_RELATIVE];
    let expected = [None, None, Some(pattern(3000, 1)), Some(pattern(2000, 9))];
    for _ in 0..2 {
        for (url, body) in urls.iter().zip(&expected) {
            let reply = fetch(&rig, url, None).await;
            assert_eq!(reply.status, 200, "{url}");
            if let Some(body) = body {
                assert_eq!(&reply.body, body, "{url}");
            }
            settle(&rig.cache).await;
        }
    }
    let stats = rig.cache.stats().await;
    assert!(stats.evictions > 0);
    assert!(stats.total_bytes <= SMALL_BUDGET);
    assert!(rig.fetcher.calls() > urls.len());
}
