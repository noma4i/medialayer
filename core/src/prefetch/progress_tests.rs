use super::*;
use crate::cache::{Cache, CacheStore, classify_kind};
use crate::fetch::{Fetcher, FetchError, Request, Response};
use crate::fetch::mock::{MockFetcher, Script};
use crate::limits::Limits;
use crate::proxy::{Config, Proxy};
use crate::proxy::memory_store::MemoryStore;
use async_trait::async_trait;
use std::{collections::HashMap, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::Duration};
use tokio::sync::Notify;

const URL: &str = "https://o.test/slow.mp4";

fn source(url: &str, kind: SourceKind) -> Source {
    Source { url: url.into(), headers: HashMap::new(), kind, strategy: CacheStrategy::Whole }
}

#[tokio::test(start_paused = true)]
async fn inv_prefetch_03_written_bytes_extend_whole_deadline() {
    for kind in [SourceKind::Image, SourceKind::Video, SourceKind::Audio] {
        let fetcher = Arc::new(MockFetcher::new());
        fetcher.route(URL, Script::ok(vec![7; 8]).chunked(1).chunk_delay(Duration::from_millis(600)));
        let store = Arc::new(MemoryStore::new());
        let proxy = Proxy::start(Config { chunk_size: 4, ..Config::default() }, fetcher.clone(), store.clone()).await.unwrap();
        let outcome = super::whole::load_whole(&ProxyLoader::new(proxy.clone()), &source(URL, kind), Duration::from_secs(1)).await;
        assert_eq!(outcome, Outcome::Loaded, "{kind:?}");
        let keys = if kind == SourceKind::Image { vec![URL.into()] } else { vec![format!("{URL}#0"), format!("{URL}#1")] };
        assert_eq!(keys.iter().map(|key| store.bytes(key).unwrap().len()).sum::<usize>(), 8);
        assert_eq!(fetcher.calls(), keys.len());
        proxy.stop().await;
    }
}

#[tokio::test(start_paused = true)]
async fn inv_prefetch_03_hls_init_and_segments_extend_deadline_on_writes() {
    let fetcher = Arc::new(MockFetcher::new());
    let playlist = "https://o.test/slow.m3u8";
    fetcher.route(playlist, Script::ok(b"#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2,\ns.ts\n#EXT-X-ENDLIST\n".to_vec()));
    for url in ["https://o.test/init.mp4", "https://o.test/s.ts"] {
        fetcher.route(url, Script::ok(vec![7; 4]).chunked(1).chunk_delay(Duration::from_millis(600)));
    }
    let store = Arc::new(MemoryStore::new());
    let proxy = Proxy::start(Config::default(), fetcher.clone(), store.clone()).await.unwrap();
    let outcome = super::whole::load_whole(&ProxyLoader::new(proxy.clone()), &source(playlist, SourceKind::Video), Duration::from_secs(1)).await;
    assert_eq!(outcome, Outcome::Loaded);
    assert_eq!(store.bytes("https://o.test/init.mp4"), Some(vec![7; 4]));
    assert_eq!(store.bytes("https://o.test/s.ts"), Some(vec![7; 4]));
    assert_eq!(fetcher.calls(), 3);
    proxy.stop().await;
}

struct ResumableOrigin {
    fetcher: MockFetcher,
    stalled: AtomicBool,
    started: Notify,
}

#[async_trait]
impl Fetcher for ResumableOrigin {
    async fn fetch(&self, request: Request) -> Result<Response, FetchError> {
        if request.range.is_some_and(|(start, _)| start > 0) && self.stalled.load(Ordering::SeqCst) {
            self.started.notify_one();
            return std::future::pending().await;
        }
        self.fetcher.fetch(request).await
    }
}

#[tokio::test]
async fn inv_strategy_07_timeout_keeps_committed_blocks_and_resumes_missing_blocks() {
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::open(directory.path(), Limits::default());
    cache.ready().await;
    let origin = Arc::new(ResumableOrigin { fetcher: MockFetcher::new(), stalled: AtomicBool::new(true), started: Notify::new() });
    origin.fetcher.route(URL, Script::ok(vec![7; 8]));
    let proxy = Proxy::start(Config { chunk_size: 4, ..Config::default() }, origin.clone(), Arc::new(CacheStore::new(cache.clone(), Arc::new(classify_kind)))).await.unwrap();
    let loader = Arc::new(ProxyLoader::new(proxy.clone()));
    let task = tokio::spawn({
        let loader = loader.clone();
        async move { super::whole::load_whole(loader.as_ref(), &source(URL, SourceKind::Video), Duration::from_secs(1)).await }
    });
    origin.started.notified().await;
    tokio::time::pause();
    assert_eq!(tokio::time::timeout(Duration::from_secs(10), task).await.unwrap().unwrap(), Outcome::TimedOut);
    tokio::time::resume();
    assert!(cache.contains(&format!("{URL}#0")).await);
    assert!(!cache.contains(&format!("{URL}#1")).await);
    origin.stalled.store(false, Ordering::SeqCst);
    assert_eq!(super::whole::load_whole(loader.as_ref(), &source(URL, SourceKind::Video), Duration::from_secs(1)).await, Outcome::Loaded);
    assert!(cache.contains(&format!("{URL}#1")).await);
    assert_eq!(origin.fetcher.requests().iter().filter(|request| request.range.is_some_and(|(start, _)| start == 0)).count(), 1);
    proxy.stop().await;
}
