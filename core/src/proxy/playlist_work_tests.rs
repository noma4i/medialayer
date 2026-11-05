use super::*;
use std::collections::HashMap;

#[tokio::test(start_paused = true)]
async fn inv_proxy_15_prefetch_refreshes_expired_live_plans() {
    use crate::prefetch::Loader;
    let store = super::super::memory_store::MemoryStore::new();
    let fetcher = Arc::new(crate::fetch::mock::MockFetcher::new());
    let target = "https://live.test/p.m3u8";
    store.seed(target, b"#EXTM3U\n#EXTINF:2,\na.ts\n");
    fetcher.route(target, crate::fetch::mock::Script::ok(b"#EXTM3U\n#EXTINF:2,\nb.ts\n".to_vec()));
    let proxy = super::super::Proxy::start(Default::default(), fetcher, Arc::new(store)).await.unwrap();
    proxy.inner.stamp_playlist(target);
    let loader = crate::prefetch::ProxyLoader::new(proxy.clone());
    assert!(loader.playlist(target, &HashMap::new()).await.unwrap().plan.chunks[0].segment.ends_with("/a.ts"));
    tokio::time::advance(std::time::Duration::from_secs(3)).await;
    loader.load(target, &HashMap::new(), super::super::Priority::Prefetch, &tokio::sync::Notify::new()).await.unwrap();
    assert!(loader.playlist(target, &HashMap::new()).await.unwrap().plan.chunks[0].segment.ends_with("/b.ts"));
    proxy.stop().await;
}

#[tokio::test]
async fn inv_proxy_15_stored_playlist_reuses_the_same_version() {
    use crate::prefetch::Loader;
    let store = super::super::memory_store::MemoryStore::new();
    let target = "https://cdn.test/p.m3u8";
    store.seed(target, b"#EXTM3U\n#EXTINF:2,\na.ts\n#EXT-X-ENDLIST\n");
    let proxy = super::super::Proxy::start(super::super::Config { namespace_headers: vec!["authorization".into()], ..Default::default() }, Arc::new(crate::fetch::mock::MockFetcher::new()), Arc::new(store.clone())).await.unwrap();
    let inner = proxy.inner.clone();
    crate::hls::work::URLS.with(|value| value.set(0));
    for _ in 0..2 {
        let response = playlist(&inner, target, HashMap::new(), false).await;
        assert_eq!(response.status(), 200);
        assert!(response.into_body().collect().await.unwrap().to_bytes().ends_with(b"#EXT-X-ENDLIST\n"));
    }
    assert_eq!(crate::hls::work::URLS.with(|value| value.get()), 1);
    let loader = crate::prefetch::ProxyLoader::new(proxy.clone());
    let prefetch = loader.playlist(target, &HashMap::new()).await.unwrap();
    let playback = proxy.playlist_plan(target, &HashMap::new()).await.unwrap();
    assert!(Arc::ptr_eq(&prefetch, &playback));
    drop((prefetch, playback));
    assert_eq!(crate::hls::work::URLS.with(|value| value.get()), 1);
    store.seed(target, b"#EXTM3U\n#EXTINF:2,\nb.ts\n#EXT-X-ENDLIST\n");
    assert!(proxy.playlist_plan(target, &HashMap::new()).await.unwrap().plan.chunks[0].segment.ends_with("/b.ts"));
    assert_eq!(crate::hls::work::URLS.with(|value| value.get()), 2);
    let headers = HashMap::from([("authorization".into(), "other".into())]);
    store.seed(&proxy.key_for(target, &headers), b"#EXTM3U\n#EXTINF:2,\nc.ts\n#EXT-X-ENDLIST\n");
    assert!(proxy.playlist_plan(target, &headers).await.unwrap().plan.chunks[0].segment.ends_with("/c.ts"));
    assert_eq!(crate::hls::work::URLS.with(|value| value.get()), 3);
    assert!(inner.config.ledger.used() > 0);
    proxy.forget_playlists(Some(target));
    assert_eq!(inner.config.ledger.used(), 0);
    drop(proxy.playlist_plan(target, &HashMap::new()).await.unwrap());
    proxy.forget_playlists(None);
    assert_eq!(inner.config.ledger.used(), 0);
    drop(proxy.playlist_plan(target, &HashMap::new()).await.unwrap());
    proxy.restart(0).await.unwrap();
    assert_eq!(inner.config.ledger.used(), 0);
    drop(proxy.playlist_plan(target, &HashMap::new()).await.unwrap());
    store.clear();
    assert!(proxy.playlist_plan(target, &HashMap::new()).await.is_none());
    assert_eq!(inner.config.ledger.used(), 0);
    proxy.stop().await;
}

#[tokio::test]
async fn inv_proxy_24_playlist_response_parses_base_and_text_once() {
    let inner = ProxyShared::new(super::super::Config::default(), Arc::new(crate::fetch::mock::MockFetcher::new()), Arc::new(super::super::memory_store::MemoryStore::new()), super::super::tasks::Tasks::default());
    let source = b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\na.ts\n#EXTINF:2,\nb.ts\n#EXT-X-ENDLIST\n";
    crate::hls::work::URLS.with(|value| value.set(0));
    crate::hls::work::PASSES.with(|value| value.set(0));
    let allocations = crate::alloc_probe::allocations_of(|| playlist_response(&inner, "https://cdn.test/p.m3u8", source, &HashMap::new(), false));
    let urls = crate::hls::work::URLS.with(|value| value.get());
    let passes = crate::hls::work::PASSES.with(|value| value.get());
    eprintln!("playlist: base_parses={urls}, text_passes={passes}, allocations={allocations}");
    assert_eq!((urls, passes), (1, 1));
}
