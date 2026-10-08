use super::*;
use crate::cache::{classify_kind, Cache, CacheStore, SystemClock};
use crate::fetch::mock::{MockFetcher, Script};
use crate::limits::Limits;
use crate::proxy::{ByteStore, Config, Proxy};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const MASTER: &str = "https://o.test/hls/master.m3u8";
const MEDIA: &str = "https://o.test/hls/v0/index.m3u8";
const INIT: &str = "https://o.test/hls/v0/init.mp4";
const SEGMENT_0: &str = "https://o.test/hls/v0/s0.m4s";
const SEGMENT_1: &str = "https://o.test/hls/v0/s1.m4s";

#[tokio::test]
async fn an_hls_source_lands_in_the_disk_cache_through_the_proxy_up_to_the_leading_depth() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), Limits::default());
    cache.ready().await;
    let store: Arc<dyn ByteStore> = Arc::new(CacheStore::new(cache.clone(), Arc::new(classify_kind)));
    let fetcher = Arc::new(MockFetcher::new());
    fetcher.route(MASTER, Script::ok(b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=900\nv0/index.m3u8\n".to_vec()));
    fetcher.route(
        MEDIA,
        Script::ok(b"#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2.0,\ns0.m4s\n#EXTINF:2.0,\ns1.m4s\n#EXT-X-ENDLIST\n".to_vec()),
    );
    fetcher.route(INIT, Script::ok(vec![1; 64]));
    fetcher.route(SEGMENT_0, Script::ok(vec![2; 4000]).chunked(1000));
    fetcher.route(SEGMENT_1, Script::ok(vec![3; 4000]));
    let proxy = Proxy::start(Config::default(), fetcher.clone(), store.clone()).await.unwrap();
    let limits = Limits::default();
    let prefetcher = Prefetcher::new(
        Arc::new(ProxyLoader::new(proxy)),
        &limits,
        PrefetchLedger::new(Duration::from_secs(60), 100),
        Arc::new(SystemClock),
    );
    let mut events = prefetcher.settled();

    let video = Source { url: MASTER.to_string(), headers: HashMap::new(), kind: SourceKind::Video };
    prefetcher.set_window(0, vec![None, None, Some(video)]);
    let settled = tokio::time::timeout(Duration::from_secs(5), events.recv()).await.unwrap().unwrap();

    assert_eq!(settled.outcome, Outcome::Loaded);
    for key in [MASTER, MEDIA, INIT, SEGMENT_0] {
        assert!(cache.contains(key).await, "{key}");
    }
    assert_eq!(cache.get(SEGMENT_0).await, Some(vec![2; 4000]));
    assert!(!cache.contains(SEGMENT_1).await);
    assert_eq!(fetcher.calls_for(SEGMENT_1), 0);
}

#[tokio::test]
async fn the_first_block_of_an_mp4_lands_as_one_ranged_block_not_the_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), Limits::default());
    cache.ready().await;
    let store: Arc<dyn ByteStore> = Arc::new(CacheStore::new(cache.clone(), Arc::new(classify_kind)));
    let fetcher = Arc::new(MockFetcher::new());
    let url = "https://o.test/clip.mp4";
    fetcher.route(url, Script::ok(vec![7; 3 * 1024 * 1024]));
    let proxy = Proxy::start(Config::default(), fetcher.clone(), store.clone()).await.unwrap();
    let loader = ProxyLoader::new(proxy);

    loader.load_first_block(url, &HashMap::new(), crate::proxy::Priority::Prefetch).await.unwrap();

    assert_eq!(fetcher.requests().iter().map(|request| request.range).collect::<Vec<_>>(), vec![Some((0, Some(1024 * 1024 - 1)))]);
    assert!(cache.contains("https://o.test/clip.mp4#0").await);
    assert!(!cache.contains(url).await);
}
