use super::test_support::proxy_rig;
use super::*;
use crate::api::Kind;
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
