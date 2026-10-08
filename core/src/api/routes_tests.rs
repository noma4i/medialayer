use super::test_support::proxy_rig;
use super::*;
use crate::api::Kind;
use crate::api::MediaError;
use std::collections::HashMap;
use std::sync::Arc;

fn video(url: &str, token: &str) -> MediaItem {
    MediaItem {
        headers: HashMap::from([("Authorization".to_string(), token.to_string())]),
        ..MediaItem::bare(url, Kind::Video)
    }
}

#[tokio::test]
async fn play_url_goes_through_the_proxy_when_it_is_ready_and_direct_when_it_is_not() {
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
async fn a_route_is_stale_after_the_proxy_changes_port_and_after_it_becomes_ready_again() {
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
async fn a_source_is_cached_when_its_whole_entry_or_its_first_block_is_stored() {
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
async fn image_bytes_load_through_the_fetcher_into_the_cache_and_refuse_a_response_over_the_limit() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    rig.fetcher.route("https://cdn.test/a.jpg", crate::fetch::mock::Script::ok(vec![1; 100]));

    let refused = routes.image_bytes("https://cdn.test/a.jpg", &HashMap::new(), 50).await;
    assert!(matches!(refused, Err(MediaError::Fetch { .. })));
    assert!(!rig.cache.contains("https://cdn.test/a.jpg").await);

    let loaded = routes.image_bytes("https://cdn.test/a.jpg", &HashMap::new(), 200).await.unwrap();
    let again = routes.image_bytes("https://cdn.test/a.jpg", &HashMap::new(), 200).await.unwrap();

    assert_eq!((loaded.len(), again.len()), (100, 100));
    assert_eq!(rig.fetcher.calls(), 2);
}

#[tokio::test]
async fn a_route_tells_the_player_whether_to_loop_and_whether_the_stream_is_hls() {
    let rig = proxy_rig().await;
    let routes = Routes::new(rig.proxy.clone(), Arc::new(rig.cache.clone()));
    let cases = [
        ("feed video", MediaItem::bare("https://cdn.test/a.mp4", Kind::Video), true, false),
        ("hls video", MediaItem::bare("https://cdn.test/a/index.m3u8", Kind::Video), true, true),
        ("voice note", MediaItem::bare("https://cdn.test/a.m4a", Kind::Audio), false, false),
    ];

    for (name, source, looping, hls) in &cases {
        let proxied = routes.play_url(source);
        assert_eq!((proxied.looping, proxied.hls), (*looping, *hls), "{name}");
    }
    rig.proxy.stop().await;
    let direct = routes.play_url(&cases[1].1);
    assert_eq!((direct.looping, direct.hls), (true, true));
}
