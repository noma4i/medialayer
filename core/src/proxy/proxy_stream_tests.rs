use super::test_client::{get, open, read_at_least, request, Reply};
use super::*;
use crate::fetch::mock::Script;
use crate::fetch::Request;
use std::time::Duration;
use super::proxy_harness::*;

async fn play_master(rig: &Rig) -> Vec<Reply> {
    let port = rig.proxy.port();
    let master_url = rig.proxy.url_for(MASTER, &HashMap::new()).expect("proxy url");
    let master = get(port, &path_of(&master_url)).await;
    assert_eq!(master.status, 200);
    let media_target = follow(&master_url, &media_lines(&master)[0]);
    let media = get(port, &media_target).await;
    assert_eq!(media.status, 200);
    let mut replies = vec![master, media];
    let media_base = format!("http://127.0.0.1:{port}{media_target}");
    for line in media_lines(&replies[1]) {
        replies.push(get(port, &follow(&media_base, &line)).await);
    }
    replies
}

#[tokio::test]
async fn inv_proxy_12_master_to_media_to_segments_streams_and_rewrites() {
    let rig = rig().await;
    hls_origin(&rig);
    let replies = play_master(&rig).await;
    assert!(replies[0].text().contains("./https://o.test/hls/media.m3u8"));
    assert!(replies[1].text().contains("./https://cdn.test/seg0.ts"));
    assert!(replies[1].text().contains("./https://o.test/hls/seg1.ts"));
    assert_eq!(replies[1].header("content-type"), Some("application/vnd.apple.mpegurl"));
    assert_eq!(replies[2].status, 200);
    assert_eq!(replies[2].body, pattern(3000, 1));
    assert_eq!(replies[2].header("content-type"), Some("video/mp2t"));
    assert_eq!(replies[3].body, pattern(2000, 9));
    assert_eq!(rig.fetcher.calls(), 4);
}

#[tokio::test]
async fn inv_proxy_03_first_pass_tees_into_the_store_and_keeps_playlists_raw() {
    let rig = rig().await;
    hls_origin(&rig);
    play_master(&rig).await;
    assert_eq!(rig.store.bytes(SEGMENT_ABSOLUTE), Some(pattern(3000, 1)));
    assert_eq!(rig.store.bytes(SEGMENT_RELATIVE), Some(pattern(2000, 9)));
    let stored_media = String::from_utf8(rig.store.bytes(MEDIA).unwrap()).unwrap();
    assert!(stored_media.contains("\nhttps://cdn.test/seg0.ts\n"));
    assert!(!stored_media.contains("./https"));
    assert_eq!(rig.store.open_writers(), 0);
}

#[tokio::test]
async fn inv_proxy_24_a_master_playlist_reaches_the_player_with_its_first_variant_only() {
    let rig = rig().await;
    let ladder = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=2000000,RESOLUTION=1280x720\nhttps://o.test/hls/media.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=6000000,RESOLUTION=1920x1080\nhttps://o.test/hls/hd.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nhttps://o.test/hls/sd.m3u8\n";
    rig.fetcher.route(MASTER, Script::ok(ladder.as_bytes().to_vec()));
    let master_url = rig.proxy.url_for(MASTER, &HashMap::new()).expect("proxy url");
    let master = get(rig.proxy.port(), &path_of(&master_url)).await;
    assert_eq!(master.status, 200);
    let text = master.text();
    assert_eq!(text.matches("#EXT-X-STREAM-INF").count(), 1, "{text}");
    assert_eq!(media_lines(&master), vec!["./https://o.test/hls/media.m3u8".to_string()]);
}

#[tokio::test]
async fn inv_proxy_12_offline_replay_serves_everything_from_the_store_then_clear_empties_it() {
    let rig = rig().await;
    hls_origin(&rig);
    let online = play_master(&rig).await;
    let calls = rig.fetcher.calls();
    rig.fetcher.set_offline(true);

    let replay = play_master(&rig).await;
    assert_eq!(rig.fetcher.calls(), calls);
    for (first, again) in online.iter().zip(&replay) {
        assert_eq!(first.body, again.body);
    }

    rig.store.clear();
    let port = rig.proxy.port();
    let after_clear = get(port, &path_of(&rig.proxy.url_for(SEGMENT_ABSOLUTE, &HashMap::new()).unwrap())).await;
    assert_eq!(after_clear.status, 502);
    assert!(after_clear.text().contains("offline"));
}

#[tokio::test]
async fn inv_proxy_11_range_requests_are_cut_from_the_store() {
    let rig = rig().await;
    let data = pattern(100, 0);
    rig.fetcher.route("https://o.test/v.mp4", Script::ok(data.clone()));
    let target = path_of(&rig.proxy.url_for("https://o.test/v.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();
    let full = get(port, &target).await;
    assert_eq!(full.status, 200);
    assert_eq!(full.header("accept-ranges"), Some("bytes"));
    assert_eq!(full.header("content-type"), Some("video/mp4"));

    let middle = request(port, "GET", &target, &[("Range", "bytes=10-19")]).await;
    assert_eq!(middle.status, 206);
    assert_eq!(middle.header("content-range"), Some("bytes 10-19/100"));
    assert_eq!(middle.header("content-length"), Some("10"));
    assert_eq!(middle.body, data[10..20]);

    let tail = request(port, "GET", &target, &[("Range", "bytes=90-")]).await;
    assert_eq!((tail.status, tail.header("content-range")), (206, Some("bytes 90-99/100")));
    assert_eq!(tail.body, data[90..]);

    let suffix = request(port, "GET", &target, &[("Range", "bytes=-5")]).await;
    assert_eq!(suffix.body, data[95..]);

    let beyond = request(port, "GET", &target, &[("Range", "bytes=100-")]).await;
    assert_eq!((beyond.status, beyond.header("content-range")), (416, Some("bytes */100")));
    assert_eq!(rig.fetcher.calls(), 1);
}

#[tokio::test]
async fn inv_proxy_19_a_range_whose_end_precedes_its_start_is_ignored_and_the_whole_body_is_sent() {
    let rig = rig().await;
    let data = pattern(100, 0);
    rig.fetcher.route("https://o.test/v.mp4", Script::ok(data.clone()));
    let target = path_of(&rig.proxy.url_for("https://o.test/v.mp4", &HashMap::new()).unwrap());

    let reply = request(rig.proxy.port(), "GET", &target, &[("Range", "bytes=5-2")]).await;

    assert_eq!((reply.status, reply.header("content-range")), (200, None));
    assert_eq!(reply.body, data);
}

#[tokio::test]
async fn inv_proxy_03_range_miss_inside_the_object_is_cached_as_blocks() {
    let rig = rig().await;
    let data = pattern(100, 0);
    rig.fetcher.route("https://o.test/v.mp4", Script::ok(data.clone()));
    let target = path_of(&rig.proxy.url_for("https://o.test/v.mp4", &HashMap::new()).unwrap());
    let reply = request(rig.proxy.port(), "GET", &target, &[("Range", "bytes=50-")]).await;
    assert_eq!((reply.status, reply.header("content-range")), (206, Some("bytes 50-99/100")));
    assert_eq!(reply.body, data[50..]);
    assert_eq!(fetched_ranges(&rig), vec![(0, Some(0)), (0, Some(99))]);
    assert_eq!(rig.store.bytes("https://o.test/v.mp4#0"), Some(data.clone()));

    let suffix = request(rig.proxy.port(), "GET", &target, &[("Range", "bytes=-10")]).await;
    assert_eq!(suffix.body, data[90..]);
    assert_eq!(rig.fetcher.calls(), 2);
}

#[tokio::test]
async fn inv_proxy_03_range_miss_from_zero_fills_the_store_and_slices_the_answer() {
    let rig = rig().await;
    let data = pattern(100, 0);
    rig.fetcher.route("https://o.test/v.mp4", Script::ok(data.clone()));
    let target = path_of(&rig.proxy.url_for("https://o.test/v.mp4", &HashMap::new()).unwrap());
    let reply = request(rig.proxy.port(), "GET", &target, &[("Range", "bytes=0-9")]).await;
    assert_eq!((reply.status, reply.header("content-range")), (206, Some("bytes 0-9/100")));
    assert_eq!(reply.body, data[..10]);
    assert_eq!(rig.fetcher.requests()[0].range, None);
    assert!(eventually(|| rig.store.bytes("https://o.test/v.mp4").is_some()).await);
}

#[tokio::test]
async fn inv_proxy_05_ten_parallel_clients_share_one_origin_call() {
    let rig = rig().await;
    rig.fetcher.route(
        "https://o.test/shared.mp4",
        Script::ok(pattern(20_000, 3)).chunked(2000).head_delay(Duration::from_millis(100)).chunk_delay(Duration::from_millis(5)),
    );
    let target = path_of(&rig.proxy.url_for("https://o.test/shared.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();
    let clients: Vec<_> = (0..10).map(|_| tokio::spawn({ let target = target.clone(); async move { get(port, &target).await } })).collect();
    for client in clients {
        let reply = within(10, client).await.unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body, pattern(20_000, 3));
    }
    assert_eq!(rig.fetcher.calls_for("https://o.test/shared.mp4"), 1);
    assert_eq!(rig.store.commits(), 1);
}

#[tokio::test]
async fn inv_proxy_08_origin_errors_pass_through_and_are_not_cached() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/gone.mp4", Script::ok(b"nope".to_vec()).status(404));
    rig.fetcher.route("https://o.test/broken.mp4", Script::ok(b"boom".to_vec()).status(503));
    let port = rig.proxy.port();
    for _ in 0..2 {
        let gone = get(port, &path_of(&rig.proxy.url_for("https://o.test/gone.mp4", &HashMap::new()).unwrap())).await;
        assert_eq!((gone.status, gone.text()), (404, "nope".to_string()));
    }
    let broken = get(port, &path_of(&rig.proxy.url_for("https://o.test/broken.mp4", &HashMap::new()).unwrap())).await;
    assert_eq!(broken.status, 503);
    assert_eq!(rig.fetcher.calls_for("https://o.test/gone.mp4"), 2);
    assert_eq!(rig.store.count(), 0);
    assert_eq!(rig.store.open_writers(), 0);
}

#[tokio::test]
async fn inv_proxy_08_fetch_errors_become_502_with_text() {
    let rig = rig().await;
    rig.fetcher.set_offline(true);
    let reply = get(rig.proxy.port(), &path_of(&rig.proxy.url_for("https://o.test/a.mp4", &HashMap::new()).unwrap())).await;
    assert_eq!(reply.status, 502);
    assert!(reply.text().contains("offline"));
}

#[tokio::test]
async fn inv_proxy_03_a_cut_origin_body_aborts_the_write_and_leaves_nothing_stored() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/cut.mp4", Script::ok(pattern(10_000, 0)).chunked(1000).cut_after(3000));
    let reply = get(rig.proxy.port(), &path_of(&rig.proxy.url_for("https://o.test/cut.mp4", &HashMap::new()).unwrap())).await;
    assert!(reply.body.len() < 10_000);
    assert!(eventually(|| rig.store.aborts() == 1).await);
    assert_eq!(rig.store.count(), 0);
    assert_eq!(rig.store.open_writers(), 0);
}

#[tokio::test]
async fn inv_proxy_08_head_answers_from_the_store_and_probes_the_origin_on_a_miss() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/v.mp4", Script::ok(pattern(100, 0)));
    let target = path_of(&rig.proxy.url_for("https://o.test/v.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();

    let miss = request(port, "HEAD", &target, &[]).await;
    assert_eq!((miss.status, miss.header("content-length")), (200, Some("100")));
    assert!(miss.body.is_empty());
    assert_eq!(rig.fetcher.requests()[0].range, Some((0, Some(0))));
    assert_eq!(rig.store.count(), 0);

    rig.store.seed("https://o.test/v.mp4", &pattern(100, 0));
    let hit = request(port, "HEAD", &target, &[]).await;
    assert_eq!((hit.status, hit.header("content-length"), hit.header("accept-ranges")), (200, Some("100"), Some("bytes")));
    assert!(hit.body.is_empty());
    assert_eq!(rig.fetcher.calls(), 1);
}

#[tokio::test]
async fn inv_proxy_08_unknown_routes_and_methods_are_rejected() {
    let rig = rig().await;
    assert_eq!(get(rig.proxy.port(), "/nothing").await.status, 404);
    assert_eq!(request(rig.proxy.port(), "POST", "/request.mp4?url=https%3A%2F%2Fo.test%2Fa.mp4", &[]).await.status, 405);
}

#[tokio::test]
async fn inv_proxy_01_only_whitelisted_headers_reach_the_origin_and_registered_headers_are_used() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/a.mp4", Script::ok(pattern(10, 0)));
    let registered = HashMap::from([("Authorization".to_string(), "Bearer registered".to_string())]);
    let target = path_of(&rig.proxy.url_for("https://o.test/a.mp4", &registered).unwrap());
    let port = rig.proxy.port();

    get(port, &target).await;
    let first: Request = rig.fetcher.requests().remove(0);
    assert_eq!(first.headers.get("Authorization").map(String::as_str), Some("Bearer registered"));

    rig.store.clear();
    request(port, "GET", &target, &[("authorization", "Bearer live"), ("X-Evil", "1"), ("User-Agent", "ua"), ("Range", "bytes=0-4")]).await;
    let second = rig.fetcher.requests().remove(1);
    assert_eq!(second.range, None);
    let names: Vec<String> = second.headers.keys().map(|name| name.to_lowercase()).collect();
    assert!(names.contains(&"user-agent".to_string()));
    assert!(!names.contains(&"x-evil".to_string()));
    assert!(!names.contains(&"range".to_string()));
    let live: Vec<&String> = second.headers.iter().filter(|(name, _)| name.eq_ignore_ascii_case("authorization")).map(|(_, value)| value).collect();
    assert_eq!(live, vec!["Bearer live"]);
}

#[tokio::test]
async fn inv_proxy_14_a_hung_origin_does_not_delay_other_answers() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/hung.mp4", Script::ok(pattern(10, 0)).hanging());
    rig.fetcher.route("https://o.test/fine.mp4", Script::ok(pattern(10, 5)));
    let port = rig.proxy.port();
    let hung_target = path_of(&rig.proxy.url_for("https://o.test/hung.mp4", &HashMap::new()).unwrap());
    let hung = tokio::spawn(async move { get(port, &hung_target).await });
    assert!(eventually(|| rig.fetcher.calls_for("https://o.test/hung.mp4") == 1).await);

    let fine = within(2, get(port, &path_of(&rig.proxy.url_for("https://o.test/fine.mp4", &HashMap::new()).unwrap()))).await;
    assert_eq!(fine.body, pattern(10, 5));
    hung.abort();
}

#[tokio::test]
async fn inv_proxy_14_a_slow_client_does_not_delay_other_clients() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/big.mp4", Script::filled(8 * 1024 * 1024).chunked(64 * 1024));
    let target = path_of(&rig.proxy.url_for("https://o.test/big.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();
    let _stalled = open(port, "GET", &target, &[]).await;
    let other = within(10, get(port, &target)).await;
    assert_eq!(other.body.len(), 8 * 1024 * 1024);
}

#[tokio::test]
async fn inv_proxy_13_a_fifty_mebibyte_segment_streams_without_buffering_it_whole() {
    let rig = rig().await;
    let size = 50 * 1024 * 1024;
    rig.fetcher.route("https://o.test/huge.mp4", Script::filled(size as u64).chunked(64 * 1024));
    let reply = within(60, get(rig.proxy.port(), &path_of(&rig.proxy.url_for("https://o.test/huge.mp4", &HashMap::new()).unwrap()))).await;
    assert_eq!(reply.body.len(), size);
    assert!(reply.body.iter().all(|byte| *byte == 0xAB));
    assert!(rig.fetcher.peak_chunks() <= PEAK_CHUNK_LIMIT, "peak chunks {}", rig.fetcher.peak_chunks());
    assert_eq!(rig.fetcher.live_chunks(), 0);
}

#[tokio::test]
async fn inv_proxy_06_a_dropped_client_cancels_the_download_and_closes_everything() {
    let rig = rig().await;
    rig.fetcher.route(
        "https://o.test/slow.mp4",
        Script::filled(10 * 1024 * 1024).chunked(16 * 1024).chunk_delay(Duration::from_millis(5)),
    );
    let target = path_of(&rig.proxy.url_for("https://o.test/slow.mp4", &HashMap::new()).unwrap());
    let mut stream = open(rig.proxy.port(), "GET", &target, &[]).await;
    assert!(read_at_least(&mut stream, 64 * 1024).await >= 64 * 1024);
    assert!(rig.proxy.live_tasks() >= 2);
    drop(stream);

    assert!(eventually(|| rig.proxy.live_tasks() == 0).await, "live tasks {}", rig.proxy.live_tasks());
    assert_eq!(rig.store.open_writers(), 0);
    assert_eq!(rig.store.aborts(), 1);
    assert_eq!(rig.store.count(), 0);
}

#[tokio::test]
async fn inv_proxy_06_one_client_leaving_does_not_cancel_a_shared_download() {
    let rig = rig().await;
    rig.fetcher.route(
        "https://o.test/shared.mp4",
        Script::ok(pattern(40_000, 2)).chunked(2000).chunk_delay(Duration::from_millis(10)),
    );
    let target = path_of(&rig.proxy.url_for("https://o.test/shared.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();
    let stayer = tokio::spawn({ let target = target.clone(); async move { get(port, &target).await } });
    let mut leaver = open(port, "GET", &target, &[]).await;
    read_at_least(&mut leaver, 1000).await;
    drop(leaver);
    let reply = within(10, stayer).await.unwrap();
    assert_eq!(reply.body, pattern(40_000, 2));
    assert_eq!(rig.fetcher.calls_for("https://o.test/shared.mp4"), 1);
    assert_eq!(rig.store.bytes("https://o.test/shared.mp4"), Some(pattern(40_000, 2)));
}
