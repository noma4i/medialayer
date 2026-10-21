use super::*;
use super::runner_tests::next;
use crate::cache::{Cache, CacheStore, SystemClock, classify_kind};
use crate::fetch::{Fetcher, FetchError, Request, Response};
use crate::limits::Limits;
use crate::proxy::{Config, Proxy};
use async_trait::async_trait;
use std::{collections::HashMap, sync::Arc, time::Duration};

struct Origin;

#[async_trait]
impl Fetcher for Origin {
    async fn fetch(&self, request: Request) -> Result<Response, FetchError> {
        let body = request.headers.iter().find(|(name, _)| name.eq_ignore_ascii_case("authorization")).unwrap().1.clone();
        Ok(Response { status: 200, headers: HashMap::from([("content-length".into(), body.len().to_string())]), body: Box::pin(futures_util::stream::once(async move { Ok(bytes::Bytes::from(body)) })) })
    }
}

fn source(token: &str) -> Source {
    Source { url: "https://origin.test/media.jpg".into(), kind: SourceKind::Image, strategy: CacheStrategy::Whole, headers: HashMap::from([("authorization".into(), token.into())]) }
}

async fn completed_namespaces(mode: &str) -> u64 {
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::open(directory.path(), Limits::default(), u64::MAX);
    cache.ready().await;
    let proxy = Proxy::start(Config { namespace_headers: vec!["authorization".into()], ..Config::default() }, Arc::new(Origin), Arc::new(CacheStore::new(cache, Arc::new(classify_kind)))).await.unwrap();
    let loader = Arc::new(ProxyLoader::new(proxy.clone()));
    let prefetch = Prefetcher::new(loader.clone(), &Limits { prefetch_from: 0, prefetch_to: 0, ..Limits::default() }, PrefetchLedger::new(Duration::from_secs(60), 100), Arc::new(SystemClock));
    let mut events = prefetch.settled();
    for token in ["A", "B"] {
        let source = Source { strategy: if mode == "whole" { CacheStrategy::Whole } else { CacheStrategy::Window }, ..source(token) };
        match mode {
            "whole" => prefetch.keep_whole(source.clone()),
            "window" => prefetch.set_window(0, vec![Some(source.clone())]),
            _ => prefetch.set_image_window("feed", 0, vec![vec![source.clone()]]),
        }
        assert_eq!(prefetch.counters().deduped, 0, "{mode}: {token}");
        assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
        assert_eq!(proxy.read_stored(&source.url, &source.headers, 1024).await, Some(token.as_bytes().to_vec()));
    }
    let mut same = source("B");
    same.strategy = if mode == "whole" { CacheStrategy::Whole } else { CacheStrategy::Window };
    same.headers = HashMap::from([("AUTHORIZATION".into(), "B".into()), ("accept".into(), "image/*".into())]);
    match mode {
        "whole" => prefetch.keep_whole(same),
        "window" => prefetch.set_window(0, vec![Some(same)]),
        _ => prefetch.set_image_window("feed", 0, vec![vec![same]]),
    }
    super::runner_tests::quiet().await;
    assert_eq!(prefetch.counters().started, 2, "{mode} repeated the same disk key");
    proxy.stop().await;
    prefetch.counters().completed
}

#[tokio::test]
async fn inv_strategy_03_namespaces_separate_completed_whole() {
    assert_eq!(completed_namespaces("whole").await, 2);
}

#[tokio::test]
async fn inv_prefetch_02_namespaces_separate_completed_window() {
    assert_eq!(completed_namespaces("window").await, 2);
}

#[tokio::test]
async fn inv_prefetch_02_namespaces_separate_completed_image_window() {
    assert_eq!(completed_namespaces("images").await, 2);
}

#[tokio::test]
async fn inv_prefetch_03_cancel_removes_all_namespaces_and_saved_declarations() {
    for mode in ["whole", "window", "images"] {
        let rig = super::runner_tests::rig_with(|limits| {
            limits.prefetch_from = 0;
            limits.prefetch_to = 1;
        });
        rig.loader.hang_all();
        let mut events = rig.prefetcher.settled();
        let sources = ["A", "B"].into_iter().map(|token| Source {
            strategy: if mode == "whole" { CacheStrategy::Whole } else { CacheStrategy::Window },
            ..source(token)
        }).collect::<Vec<_>>();
        match mode {
            "whole" => sources.iter().for_each(|source| rig.prefetcher.keep_whole(source.clone())),
            "window" => rig.prefetcher.set_window(0, sources.into_iter().map(Some).collect::<Vec<_>>()),
            _ => rig.prefetcher.set_image_window("feed", 0, vec![sources]),
        }
        super::runner_tests::quiet().await;
        assert_eq!(rig.loader.active(), 2, "{mode}");
        rig.prefetcher.cancel(&source("A").url);
        for _ in 0..2 {
            assert_eq!(next(&mut events).await.outcome, Outcome::Cancelled);
        }
        rig.prefetcher.resume();
        super::runner_tests::quiet().await;
        assert_eq!(rig.loader.calls().len(), 2, "cancelled {mode} resumed");
        assert_eq!(rig.loader.active(), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn inv_prefetch_03_whole_stalled_loads_time_out_by_kind_and_hls_part() {
    let mut failures = Vec::new();
    for (kind, url, hanging) in [
        (SourceKind::Image, "https://o.test/i.jpg", "https://o.test/i.jpg"),
        (SourceKind::Video, "https://o.test/v.mp4", "https://o.test/v.mp4"),
        (SourceKind::Audio, "https://o.test/a.mp4", "https://o.test/a.mp4"),
        (SourceKind::Video, "https://o.test/v.m3u8", "https://o.test/init.mp4"),
        (SourceKind::Video, "https://o.test/v.m3u8", "https://o.test/s.ts"),
    ] {
        let rig = super::runner_tests::rig_with(|limits| limits.timeout_seconds = 1);
        rig.loader.serve(url, "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2,\ns.ts\n#EXT-X-ENDLIST\n");
        rig.loader.hang(hanging);
        let mut events = rig.prefetcher.settled();
        rig.prefetcher.keep_whole(Source { url: url.into(), kind, strategy: CacheStrategy::Whole, headers: HashMap::new() });
        let result = tokio::time::timeout(Duration::from_secs(10), events.recv()).await;
        if !matches!(result, Ok(Ok(ref event)) if event.outcome == Outcome::TimedOut) {
            failures.push(format!("{kind:?} {hanging}: {result:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

struct HlsOrigin;

#[async_trait]
impl Fetcher for HlsOrigin {
    async fn fetch(&self, request: Request) -> Result<Response, FetchError> {
        let body = if request.url.ends_with("m3u8") {
            b"#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2,\ns.ts\n#EXT-X-ENDLIST\n".to_vec()
        } else if request.url.ends_with("init.mp4") {
            vec![1; 1024]
        } else {
            vec![2; 700 * 1024]
        };
        Ok(Response { status: 200, headers: HashMap::from([("content-length".into(), body.len().to_string())]), body: Box::pin(futures_util::stream::once(async move { Ok(bytes::Bytes::from(body)) })) })
    }
}

#[tokio::test]
async fn inv_prefetch_12_played_and_evicted_parts_use_their_namespace() {
    let directory = tempfile::tempdir().unwrap();
    let cache = Cache::open(directory.path(), Limits::default(), u64::MAX);
    cache.ready().await;
    let proxy = Proxy::start(Config { namespace_headers: vec!["authorization".into()], ..Config::default() }, Arc::new(HlsOrigin), Arc::new(CacheStore::new(cache, Arc::new(classify_kind)))).await.unwrap();
    let loader = Arc::new(ProxyLoader::new(proxy.clone()));
    let prefetch = Prefetcher::new(loader.clone(), &Limits { prefetch_from: 1, prefetch_to: 2, prefetch_budget_mib: 1, ..Limits::default() }, PrefetchLedger::new(Duration::from_secs(60), 100), Arc::new(SystemClock));
    let mut events = prefetch.settled();
    let a = Source { url: "https://origin.test/v.m3u8".into(), kind: SourceKind::Video, strategy: CacheStrategy::Window, ..source("A") };
    let b = Source { headers: source("B").headers, ..a.clone() };
    prefetch.set_window(0, vec![None, None, Some(a.clone())]);
    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    assert_eq!(prefetch.held_bytes(), 701 * 1024);
    prefetch.set_window(0, vec![Some(b.clone()), None, Some(a.clone())]);
    assert_eq!(prefetch.held_bytes(), 701 * 1024, "playing B must not release A accounting");
    prefetch.set_window(0, vec![None, Some(b.clone()), Some(a.clone())]);
    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    for part in ["https://origin.test/init.mp4", "https://origin.test/s.ts"] {
        assert_eq!(loader.stored_len(part, &a.headers).await, None, "A {part}");
        assert!(loader.stored_len(part, &b.headers).await.is_some(), "B {part}");
    }
    assert_eq!(prefetch.held_bytes(), 701 * 1024);
    prefetch.set_window(0, vec![Some(b), None, Some(a)]);
    assert_eq!(prefetch.held_bytes(), 0, "playing B releases only B accounting");
    proxy.stop().await;
}
