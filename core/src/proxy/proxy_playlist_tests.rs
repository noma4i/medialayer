use super::test_client::{get, request, Reply};
use super::*;
use crate::fetch::mock::Script;
use std::time::Duration;
use super::proxy_harness::*;

const FMP4_MEDIA: &str = "https://o.test/f/media.m3u8";

fn fmp4_origin(rig: &Rig) {
    rig.fetcher.route(
        FMP4_MEDIA,
        Script::ok(
            b"#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"https://keys.test/k1\"\n#EXTINF:2.0,\nseg0.m4s\n#EXT-X-ENDLIST\n".to_vec(),
        ),
    );
    rig.fetcher.route("https://o.test/f/init.mp4", Script::ok(pattern(300, 2)));
    rig.fetcher.route("https://keys.test/k1", Script::ok(pattern(16, 5)));
    rig.fetcher.route("https://o.test/f/seg0.m4s", Script::ok(pattern(900, 8)));
}

fn tag_uris(playlist: &str) -> Vec<String> {
    playlist
        .lines()
        .filter(|line| line.starts_with('#'))
        .filter_map(|line| line.split_once("URI=\"").and_then(|(_, rest)| rest.split_once('"')).map(|(uri, _)| uri.to_string()))
        .collect()
}

async fn walk_fmp4(rig: &Rig) -> Vec<Reply> {
    let port = rig.proxy.port();
    let media_url = rig.proxy.url_for(FMP4_MEDIA, &HashMap::new()).unwrap();
    let playlist = get(port, &path_of(&media_url)).await;
    assert_eq!(playlist.status, 200);
    let mut replies = Vec::new();
    for uri in tag_uris(&playlist.text()).into_iter().chain(media_lines(&playlist)) {
        let reply = get(port, &follow(&media_url, &uri)).await;
        assert_eq!(reply.status, 200, "{uri}");
        replies.push(reply);
    }
    replies.insert(0, playlist);
    replies
}

#[tokio::test]
async fn inv_proxy_12_fmp4_playlist_routes_map_and_key_uris_through_the_proxy() {
    let rig = rig().await;
    fmp4_origin(&rig);
    let replies = walk_fmp4(&rig).await;
    let playlist = replies[0].text();
    assert!(playlist.contains("URI=\"./https://o.test/f/init.mp4\""), "{playlist}");
    assert!(playlist.contains("URI=\"./https://keys.test/k1\""), "{playlist}");
    assert_eq!(replies.len(), 4);
    assert_eq!(replies[1].body, pattern(300, 2));
    assert_eq!(replies[2].body, pattern(16, 5));
    assert_eq!(replies[3].body, pattern(900, 8));
    assert_eq!(rig.store.bytes("https://o.test/f/init.mp4"), Some(pattern(300, 2)));
    assert_eq!(rig.store.bytes("https://keys.test/k1"), Some(pattern(16, 5)));
}

#[tokio::test]
async fn inv_proxy_12_fmp4_playlist_replays_offline_including_init_and_key() {
    let rig = rig().await;
    fmp4_origin(&rig);
    let online = walk_fmp4(&rig).await;
    let calls = rig.fetcher.calls();
    rig.fetcher.set_offline(true);
    let replay = walk_fmp4(&rig).await;
    assert_eq!(rig.fetcher.calls(), calls);
    for (first, again) in online.iter().zip(&replay) {
        assert_eq!(first.body, again.body);
    }
}

fn live_playlist(sequence: u32) -> Vec<u8> {
    format!("#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:{sequence}\n#EXTINF:2.0,\nseg{sequence}.ts\n").into_bytes()
}

#[tokio::test]
async fn inv_proxy_13_a_playlist_over_the_input_limit_is_refused_with_502() {
    let mut config = config(8);
    config.max_playlist_bytes = 64;
    let rig = rig_with(config).await;
    let url = "https://o.test/huge.m3u8";
    rig.fetcher.route(url, Script::ok(format!("#EXTM3U\n{}", "#EXT-X-COMMENT:padding\n".repeat(20)).into_bytes()));
    let target = path_of(&rig.proxy.url_for(url, &HashMap::new()).unwrap());

    let reply = get(rig.proxy.port(), &target).await;

    assert_eq!((reply.status, reply.text().as_str()), (502, "playlist too large"));
}

#[tokio::test(start_paused = true)]
async fn inv_proxy_12_live_playlists_are_refetched_after_the_ttl_and_vod_playlists_are_not() {
    let mut config = config(8);
    config.playlist_ttl = Duration::from_millis(150);
    let rig = rig_with(config).await;
    let live = "https://o.test/live.m3u8";
    let vod = "https://o.test/vod.m3u8";
    rig.fetcher.route(live, Script::ok(live_playlist(1)));
    rig.fetcher.route(vod, Script::ok(b"#EXTM3U\n#EXTINF:2.0,\nseg0.ts\n#EXT-X-ENDLIST\n".to_vec()));
    let port = rig.proxy.port();
    let live_target = path_of(&rig.proxy.url_for(live, &HashMap::new()).unwrap());
    let vod_target = path_of(&rig.proxy.url_for(vod, &HashMap::new()).unwrap());

    assert!(get(port, &live_target).await.text().contains("MEDIA-SEQUENCE:1"));
    assert!(get(port, &live_target).await.text().contains("MEDIA-SEQUENCE:1"));
    get(port, &vod_target).await;
    assert_eq!(rig.fetcher.calls_for(live), 1);

    rig.fetcher.route(live, Script::ok(live_playlist(2)));
    tokio::time::advance(Duration::from_millis(300)).await;
    assert!(get(port, &live_target).await.text().contains("MEDIA-SEQUENCE:2"));
    get(port, &vod_target).await;
    assert_eq!(rig.fetcher.calls_for(live), 2);
    assert_eq!(rig.fetcher.calls_for(vod), 1);
}

#[tokio::test]
async fn inv_proxy_12_a_saved_content_type_is_served_from_the_store_instead_of_a_guess() {
    let rig = rig().await;
    let url = "https://o.test/clip.mp4";
    rig.fetcher.route(url, Script::ok(pattern(100, 0)).header("Content-Type", "video/x-special"));
    let target = path_of(&rig.proxy.url_for(url, &HashMap::new()).unwrap());
    let port = rig.proxy.port();
    assert_eq!(get(port, &target).await.header("content-type"), Some("video/x-special"));
    assert_eq!(
        rig.store.meta(url).await,
        Some(Meta { content_type: Some("video/x-special".to_string()), total_length: Some(100) })
    );

    rig.fetcher.set_offline(true);
    let replay = get(port, &target).await;
    assert_eq!(replay.header("content-type"), Some("video/x-special"));
    let ranged = request(port, "GET", &target, &[("Range", "bytes=0-9")]).await;
    assert_eq!(ranged.header("content-type"), Some("video/x-special"));
}
