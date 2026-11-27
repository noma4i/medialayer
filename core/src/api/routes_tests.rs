use super::test_support::proxy_rig;
use super::*;
use crate::api::Kind;
use crate::api::MediaError;
use std::collections::HashMap;
use std::sync::Arc;
use url::Url;

fn video(url: &str, token: &str) -> MediaItem {
    MediaItem {
        headers: HashMap::from([("Authorization".to_string(), token.to_string())]),
        ..MediaItem::bare(url, Kind::Video)
    }
}

#[tokio::test]
async fn inv_api_08_play_url_goes_through_the_proxy_when_it_is_ready_and_direct_when_it_is_not() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    let source = video("https://cdn.test/a/index.m3u8", "token");

    let old_port = rig.proxy.port();
    let proxied = routes.play_url(&source);
    let file = routes.play_url(&MediaItem::bare("file:///clip.mp4", Kind::Video));
    rig.proxy.stop().await;
    let direct = routes.play_url(&source);

    assert!(proxied.url.starts_with(&format!("http://127.0.0.1:{}/", old_port)), "{}", proxied.url);
    assert_eq!((proxied.headers.len(), proxied.generation), (0, Some(0)));
    assert_eq!((file.url.as_str(), file.generation), ("file:///clip.mp4", None));
    assert_eq!((direct.url.as_str(), direct.headers["Authorization"].as_str(), direct.generation), ("https://cdn.test/a/index.m3u8", "token", None));
}

#[tokio::test]
async fn inv_api_08_a_route_is_stale_after_the_proxy_changes_port_and_after_it_becomes_ready_again() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    let source = video("https://cdn.test/a/index.m3u8", "token");
    let first = routes.play_url(&source);
    assert!(!routes.is_stale(&first));
    assert_eq!(routes.refresh(&source, &first), first);

    let old_port = rig.proxy.port();
    while rig.proxy.port() == old_port {
        rig.proxy.restart(0).await.unwrap();
    }
    let second = routes.refresh(&source, &first);

    assert!(routes.is_stale(&first));
    assert_ne!(second.url, first.url);
    assert_eq!(second.generation, Some(rig.proxy.route_generation()));
    rig.proxy.stop().await;
    let direct = routes.play_url(&source);
    rig.proxy.restart(0).await.unwrap();
    assert!(routes.is_stale(&direct));
}

#[tokio::test]
async fn inv_heal_route_refresh_after_stop_preserves_origin_headers() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    let source = video("https://cdn.test/clip.mp4", "token");
    let proxied = routes.play_url(&source);
    rig.proxy.stop().await;
    let direct = routes.refresh(&source, &proxied);
    assert_eq!((direct.url, direct.headers, direct.generation), (source.url, source.headers, None));
}

#[tokio::test]
async fn inv_heal_route_with_a_different_port_is_stale() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    let mut route = routes.play_url(&video("https://cdn.test/clip.mp4", "token"));
    let mut url = Url::parse(&route.url).unwrap();
    url.set_port(Some(if rig.proxy.port() == 1 { 2 } else { 1 })).unwrap();
    route.url = url.to_string();
    assert_eq!(routes.is_stale(&route), true);
}

#[tokio::test]
async fn inv_api_08_a_source_is_cached_when_its_whole_entry_or_its_first_block_is_stored() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    let source = video("https://cdn.test/clip.mp4", "token");
    let other = video("https://cdn.test/other.mp4", "token");
    assert!(!routes.is_cached(&source.url, &source.headers).await);

    rig.cache.put("https://cdn.test/clip.mp4#0", crate::cache::Kind::Stream, b"first block").await.unwrap();
    rig.cache.put("https://cdn.test/other.mp4", crate::cache::Kind::Stream, b"whole").await.unwrap();

    assert!(routes.is_cached(&source.url, &source.headers).await);
    assert!(routes.is_cached(&other.url, &other.headers).await);
    assert!(!routes.is_cached("https://cdn.test/third.mp4", &HashMap::new()).await);
}

#[tokio::test]
async fn inv_ffi_03_image_bytes_load_through_the_fetcher_into_the_cache_and_refuse_a_response_over_the_limit() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    rig.fetcher.route("https://cdn.test/a.jpg", crate::fetch::mock::Script::ok(vec![1; 100]));

    let refused = routes.image_bytes("https://cdn.test/a.jpg", &HashMap::new(), 50).await;
    assert!(matches!(refused, Err(MediaError::Fetch { .. })));
    assert!(!rig.cache.contains("https://cdn.test/a.jpg").await);

    let loaded = routes.image_bytes("https://cdn.test/a.jpg", &HashMap::new(), 200).await.unwrap();
    let again = routes.image_bytes("https://cdn.test/a.jpg", &HashMap::new(), 200).await.unwrap();

    assert_eq!((loaded.bytes.len(), again.bytes.len()), (100, 100));
    assert_eq!(rig.fetcher.calls(), 2);
}

#[tokio::test]
async fn inv_api_08_inv_api_12_a_route_tells_the_player_whether_to_loop_and_whether_the_stream_is_hls() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    let cases = [
        ("feed video", MediaItem::bare("https://cdn.test/a.mp4", Kind::Video), true, false),
        ("hls video", MediaItem::bare("https://cdn.test/a/index.m3u8", Kind::Video), true, true),
        ("voice note", MediaItem::bare("https://cdn.test/a.m4a", Kind::Audio), false, false),
        ("one-shot intro", MediaItem { looping: false, ..MediaItem::bare("https://cdn.test/intro.mp4", Kind::Video) }, false, false),
    ];

    for (name, source, looping, hls) in &cases {
        let proxied = routes.play_url(source);
        assert_eq!((proxied.looping, proxied.hls), (*looping, *hls), "{name}");
    }
    rig.proxy.stop().await;
    let direct = routes.play_url(&cases[1].1);
    assert_eq!((direct.looping, direct.hls), (true, true));
}

#[tokio::test]
async fn inv_api_15_a_local_file_is_cached_by_definition_and_is_played_from_its_own_path() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    let clip = MediaItem::bare("file:///bundle/intro.mp4", Kind::Video);

    let route = routes.play_url(&clip);

    assert!(clip.is_local() && !MediaItem::bare("https://cdn.test/a.mp4", Kind::Video).is_local());
    assert!(routes.is_cached(&clip.url, &clip.headers).await);
    assert_eq!((route.url.as_str(), route.generation, route.headers.len()), ("file:///bundle/intro.mp4", None, 0));
    assert_eq!(rig.fetcher.calls(), 0);
}

#[tokio::test]
async fn inv_image_02_a_file_image_is_read_from_its_path_and_neither_asks_the_fetcher_nor_enters_the_cache() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("a photo.jpg");
    std::fs::write(&path, vec![7u8; 100]).unwrap();
    let url = url::Url::from_file_path(&path).unwrap().to_string();
    let missing = url::Url::from_file_path(folder.path().join("gone.jpg")).unwrap().to_string();

    let read = routes.image_bytes(&url, &HashMap::new(), 200).await;
    let over = routes.image_bytes(&url, &HashMap::new(), 99).await;
    let absent = routes.image_bytes(&missing, &HashMap::new(), 200).await;
    let exact = routes.image_bytes(&url, &HashMap::new(), 100).await;

    assert_eq!(read.unwrap().bytes, vec![7u8; 100]);
    assert_eq!(exact.unwrap().bytes, vec![7u8; 100]);
    assert!(matches!(over, Err(MediaError::Fetch { .. })));
    assert!(matches!(absent, Err(MediaError::Fetch { .. })));
    assert_eq!(rig.fetcher.calls(), 0);
    assert_eq!(rig.cache.stats().await.file_count, 0);
}
