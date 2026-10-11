use super::memory_store::{Failure, MemoryStore};
use super::test_client::{get, open, read_at_least, request, Reply};
use super::*;
use crate::fetch::mock::{MockFetcher, Script};
use crate::fetch::Request;
use std::future::Future;
use std::time::Duration;
use url::Url;

const MASTER: &str = "https://o.test/hls/master.m3u8";
const MEDIA: &str = "https://o.test/hls/media.m3u8";
const SEGMENT_ABSOLUTE: &str = "https://cdn.test/seg0.ts";
const SEGMENT_RELATIVE: &str = "https://o.test/hls/seg1.ts";
const PEAK_CHUNK_LIMIT: usize = 8;

struct Rig {
    proxy: Proxy,
    fetcher: Arc<MockFetcher>,
    store: MemoryStore,
}

fn config(max_concurrent: usize) -> Config {
    let mut config = Config::default();
    config.max_concurrent = max_concurrent;
    config.policy = config.policy.with_timeout(Duration::from_secs(5));
    config
}

async fn rig() -> Rig {
    rig_with(config(8)).await
}

async fn rig_with(config: Config) -> Rig {
    let fetcher = Arc::new(MockFetcher::new());
    let store = MemoryStore::new();
    let proxy = Proxy::start(config, fetcher.clone(), Arc::new(store.clone())).await.expect("proxy start");
    Rig { proxy, fetcher, store }
}

fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|index| (index as u8).wrapping_add(seed)).collect()
}

fn hls_origin(rig: &Rig) {
    rig.fetcher.route(MASTER, Script::ok(b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\nhttps://o.test/hls/media.m3u8\n".to_vec()));
    rig.fetcher.route(
        MEDIA,
        Script::ok(
            b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\nhttps://cdn.test/seg0.ts\n#EXTINF:2.0,\nseg1.ts\n#EXT-X-ENDLIST\n".to_vec(),
        ),
    );
    rig.fetcher.route(SEGMENT_ABSOLUTE, Script::ok(pattern(3000, 1)).chunked(700));
    rig.fetcher.route(SEGMENT_RELATIVE, Script::ok(pattern(2000, 9)).chunked(700));
}

fn path_of(proxy_url: &str) -> String {
    let parsed = Url::parse(proxy_url).unwrap();
    format!("{}?{}", parsed.path(), parsed.query().unwrap_or_default())
}

fn follow(base: &str, line: &str) -> String {
    let resolved = Url::parse(base).unwrap().join(line).unwrap();
    match resolved.query() {
        Some(query) => format!("{}?{query}", resolved.path()),
        None => resolved.path().to_string(),
    }
}

fn media_lines(reply: &Reply) -> Vec<String> {
    reply.text().lines().filter(|line| !line.starts_with('#') && !line.is_empty()).map(str::to_string).collect()
}

async fn eventually(mut condition: impl FnMut() -> bool) -> bool {
    for _ in 0..300 {
        if condition() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    condition()
}

async fn within<T>(seconds: u64, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(seconds), future).await.expect("timed out")
}

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

#[tokio::test]
async fn inv_proxy_09_playback_starts_while_prefetch_holds_every_prefetch_slot() {
    let rig = rig_with(config(2)).await;
    rig.fetcher.route("https://o.test/low1.mp4", Script::ok(pattern(10, 0)).hanging());
    rig.fetcher.route("https://o.test/low2.mp4", Script::ok(pattern(10, 0)).hanging());
    rig.fetcher.route("https://o.test/now.mp4", Script::ok(pattern(10, 7)));
    let first = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/low1.mp4", &HashMap::new(), Priority::Prefetch).await } });
    assert!(eventually(|| rig.fetcher.calls_for("https://o.test/low1.mp4") == 1).await);
    let second = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/low2.mp4", &HashMap::new(), Priority::Prefetch).await } });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.fetcher.calls_for("https://o.test/low2.mp4"), 0);

    let now = within(2, get(rig.proxy.port(), &path_of(&rig.proxy.url_for("https://o.test/now.mp4", &HashMap::new()).unwrap()))).await;
    assert_eq!(now.body, pattern(10, 7));
    first.abort();
    second.abort();
}

#[tokio::test]
async fn inv_proxy_03_prefetch_fills_the_store_and_later_playback_works_offline() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/p.mp4", Script::ok(pattern(5000, 6)).chunked(1000));
    rig.proxy.fetch_into_store("https://o.test/p.mp4", &HashMap::new(), Priority::Prefetch).await.unwrap();
    assert_eq!(rig.store.bytes("https://o.test/p.mp4"), Some(pattern(5000, 6)));

    rig.proxy.fetch_into_store("https://o.test/p.mp4", &HashMap::new(), Priority::Prefetch).await.unwrap();
    assert_eq!(rig.fetcher.calls(), 1);
    rig.fetcher.set_offline(true);
    let reply = get(rig.proxy.port(), &path_of(&rig.proxy.url_for("https://o.test/p.mp4", &HashMap::new()).unwrap())).await;
    assert_eq!(reply.body, pattern(5000, 6));
}

#[tokio::test]
async fn inv_proxy_08_prefetch_reports_origin_status_and_fetch_failures() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/gone.mp4", Script::ok(b"x".to_vec()).status(404));
    assert_eq!(rig.proxy.fetch_into_store("https://o.test/gone.mp4", &HashMap::new(), Priority::Prefetch).await, Err(ProxyError::Status(404)));
    rig.fetcher.set_offline(true);
    let failure = rig.proxy.fetch_into_store("https://o.test/other.mp4", &HashMap::new(), Priority::Prefetch).await;
    assert!(matches!(failure, Err(ProxyError::Fetch(_))));
    rig.fetcher.set_offline(false);
    rig.store.fail_at(Some(Failure::Begin));
    rig.fetcher.route("https://o.test/full.mp4", Script::ok(pattern(10, 0)));
    assert_eq!(rig.proxy.fetch_into_store("https://o.test/full.mp4", &HashMap::new(), Priority::Prefetch).await, Err(ProxyError::StoreRefused));
}

#[tokio::test]
async fn inv_proxy_17_restart_on_another_port_raises_the_route_generation() {
    let rig = rig().await;
    assert_eq!(rig.proxy.route_generation(), 0);
    let first = rig.proxy.port();
    let spare = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    assert_eq!(rig.proxy.restart(spare).await, Ok(spare));
    assert_eq!(rig.proxy.route_generation(), 1);
    assert_ne!(spare, first);

    assert_eq!(rig.proxy.restart(spare).await, Ok(spare));
    assert_eq!(rig.proxy.route_generation(), 1);

    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let busy = occupied.local_addr().unwrap().port();
    let fallback = rig.proxy.restart(busy).await.unwrap();
    assert_ne!(fallback, busy);
    assert_eq!(rig.proxy.port(), fallback);
    assert_eq!(rig.proxy.route_generation(), 2);
    assert!(rig.proxy.ping().await);
}

#[tokio::test]
async fn inv_proxy_17_ping_and_readiness_follow_the_lifecycle() {
    let rig = rig().await;
    assert_eq!(rig.proxy.readiness(), Readiness::Ready);
    assert!(rig.proxy.ping().await);
    assert!(rig.proxy.url_for("https://o.test/a.mp4", &HashMap::new()).unwrap().starts_with(&format!("http://127.0.0.1:{}/", rig.proxy.port())));
    let port = rig.proxy.port();

    rig.proxy.stop().await;
    assert_eq!(rig.proxy.readiness(), Readiness::Stopped);
    assert!(!rig.proxy.ping().await);
    assert_eq!(rig.proxy.url_for("https://o.test/a.mp4", &HashMap::new()), None);
    assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err());
    assert_eq!(rig.proxy.live_tasks(), 0);

    rig.proxy.restart(0).await.unwrap();
    assert_eq!(rig.proxy.readiness(), Readiness::Ready);
    assert!(rig.proxy.ping().await);
}

#[tokio::test]
async fn inv_proxy_23_a_listener_closed_under_the_accept_loop_is_rebound_by_repair() {
    let rig = rig().await;
    let port = rig.proxy.port();
    super::test_client::close_listener_silently(port);

    assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err());
    assert_eq!(rig.proxy.readiness(), Readiness::Ready);
    assert!(!rig.proxy.ping().await);

    assert!(rig.proxy.repair().await);

    assert!(rig.proxy.ping().await);
    rig.fetcher.route("https://o.test/a.mp4", Script::ok(pattern(10, 3)));
    let url = rig.proxy.url_for("https://o.test/a.mp4", &HashMap::new()).unwrap();
    let reply = get(rig.proxy.port(), &path_of(&url)).await;
    assert_eq!(reply.body, pattern(10, 3));
}

#[tokio::test]
async fn inv_proxy_23_an_accept_task_that_ended_is_rebound_by_repair() {
    let rig = rig().await;
    let port = rig.proxy.port();
    let _ = rig.proxy.inner.listener.lock().await.as_ref().unwrap().shutdown.send(true);
    while rig.proxy.readiness() == Readiness::Ready {
        tokio::task::yield_now().await;
    }
    assert_eq!(rig.proxy.readiness(), Readiness::Unhealthy);
    assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err());

    assert!(rig.proxy.repair().await);

    assert_eq!(rig.proxy.readiness(), Readiness::Ready);
    assert!(rig.proxy.ping().await);
}

#[tokio::test]
async fn inv_proxy_23_repair_leaves_a_healthy_or_stopped_proxy_alone() {
    let rig = rig().await;
    let (port, generation) = (rig.proxy.port(), rig.proxy.route_generation());
    assert!(rig.proxy.repair().await);
    assert_eq!((rig.proxy.port(), rig.proxy.route_generation()), (port, generation));

    rig.proxy.stop().await;
    assert!(!rig.proxy.repair().await);
    assert_eq!(rig.proxy.readiness(), Readiness::Stopped);
}

#[tokio::test]
async fn inv_proxy_07_stop_closes_open_connections_and_cancels_their_downloads() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/slow.mp4", Script::filled(10 * 1024 * 1024).chunked(16 * 1024).chunk_delay(Duration::from_millis(5)));
    let target = path_of(&rig.proxy.url_for("https://o.test/slow.mp4", &HashMap::new()).unwrap());
    let mut stream = open(rig.proxy.port(), "GET", &target, &[]).await;
    read_at_least(&mut stream, 32 * 1024).await;
    rig.proxy.stop().await;
    assert!(eventually(|| rig.proxy.live_tasks() == 0).await);
    assert_eq!(rig.store.open_writers(), 0);
}

fn fetched_ranges(rig: &Rig) -> Vec<(u64, Option<u64>)> {
    rig.fetcher.requests().iter().filter_map(|request| request.range).collect()
}

fn block_config(block: u64) -> Config {
    let mut config = config(8);
    config.chunk_size = block;
    config
}

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

#[tokio::test]
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
    tokio::time::sleep(Duration::from_millis(300)).await;
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

#[tokio::test]
async fn inv_proxy_05_overlapping_ranges_fetch_only_the_missing_blocks() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(4500, 1);
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(data.clone()).chunked(300));
    let target = path_of(&rig.proxy.url_for("https://o.test/r.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();

    let first = request(port, "GET", &target, &[("Range", "bytes=1500-2499")]).await;
    assert_eq!((first.status, first.header("content-range")), (206, Some("bytes 1500-2499/4500")));
    assert_eq!(first.body, data[1500..2500]);
    assert_eq!(fetched_ranges(&rig), vec![(0, Some(0)), (1000, Some(1999)), (2000, Some(2999))]);
    let after_first = rig.fetcher.bytes_served();
    assert_eq!(after_first, 2000);

    let second = request(port, "GET", &target, &[("Range", "bytes=2000-3499")]).await;
    assert_eq!(second.body, data[2000..3500]);
    assert_eq!(fetched_ranges(&rig).last(), Some(&(3000, Some(3999))));
    assert_eq!(rig.fetcher.bytes_served() - after_first, 1000);

    let again = request(port, "GET", &target, &[("Range", "bytes=1500-3499")]).await;
    assert_eq!(again.body, data[1500..3500]);
    assert_eq!(rig.fetcher.bytes_served() - after_first, 1000);
    assert_eq!(rig.store.bytes("https://o.test/r.mp4#2"), Some(data[2000..3000].to_vec()));
    assert_eq!(rig.store.meta("https://o.test/r.mp4#1").await.unwrap().total_length, Some(4500));
}

#[tokio::test]
async fn inv_proxy_11_suffix_and_tail_ranges_use_a_short_last_block() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(4500, 3);
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(data.clone()));
    let target = path_of(&rig.proxy.url_for("https://o.test/r.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();
    let suffix = request(port, "GET", &target, &[("Range", "bytes=-300")]).await;
    assert_eq!((suffix.status, suffix.header("content-range")), (206, Some("bytes 4200-4499/4500")));
    assert_eq!(suffix.body, data[4200..]);
    assert_eq!(fetched_ranges(&rig).last(), Some(&(4000, Some(4499))));
    let beyond = request(port, "GET", &target, &[("Range", "bytes=9999-")]).await;
    assert_eq!((beyond.status, beyond.header("content-range")), (416, Some("bytes */4500")));
}

#[tokio::test]
async fn inv_proxy_05_parallel_ranges_over_one_block_fetch_it_once() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(3000, 4);
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(data.clone()).chunked(100).chunk_delay(Duration::from_millis(5)));
    let target = path_of(&rig.proxy.url_for("https://o.test/r.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();
    let clients: Vec<_> = (0..4)
        .map(|_| tokio::spawn({ let target = target.clone(); async move { request(port, "GET", &target, &[("Range", "bytes=1100-1900")]).await } }))
        .collect();
    for client in clients {
        assert_eq!(within(10, client).await.unwrap().body, data[1100..1901]);
    }
    assert_eq!(fetched_ranges(&rig).iter().filter(|range| range.0 == 1000).count(), 1);
}

#[tokio::test]
async fn inv_proxy_09_prefetch_over_the_wait_queue_limit_is_refused_at_once() {
    let mut config = config(2);
    config.max_waiting = 1;
    let rig = rig_with(config).await;
    for name in ["a", "b", "c"] {
        rig.fetcher.route(&format!("https://o.test/{name}.mp4"), Script::ok(pattern(10, 0)).hanging());
    }
    let running = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/a.mp4", &HashMap::new(), Priority::Prefetch).await } });
    assert!(eventually(|| rig.fetcher.calls_for("https://o.test/a.mp4") == 1).await);
    let waiting = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/b.mp4", &HashMap::new(), Priority::Prefetch).await } });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let refused = within(1, rig.proxy.fetch_into_store("https://o.test/c.mp4", &HashMap::new(), Priority::Prefetch)).await;
    assert_eq!(refused, Err(ProxyError::QueueFull));
    assert_eq!(rig.fetcher.calls_for("https://o.test/c.mp4"), 0);
    running.abort();
    waiting.abort();
}

#[tokio::test]
async fn inv_proxy_09_a_player_request_promotes_a_waiting_prefetch() {
    let rig = rig_with(config(2)).await;
    rig.fetcher.route("https://o.test/hold.mp4", Script::ok(pattern(10, 0)).hanging());
    rig.fetcher.route("https://o.test/next.mp4", Script::ok(pattern(10, 6)));
    let hold = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/hold.mp4", &HashMap::new(), Priority::Prefetch).await } });
    assert!(eventually(|| rig.fetcher.calls_for("https://o.test/hold.mp4") == 1).await);
    let queued = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/next.mp4", &HashMap::new(), Priority::Prefetch).await } });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.fetcher.calls_for("https://o.test/next.mp4"), 0);

    let target = path_of(&rig.proxy.url_for("https://o.test/next.mp4", &HashMap::new()).unwrap());
    let reply = within(2, get(rig.proxy.port(), &target)).await;
    assert_eq!(reply.body, pattern(10, 6));
    assert_eq!(rig.fetcher.calls_for("https://o.test/next.mp4"), 1);
    assert_eq!(within(2, queued).await.unwrap(), Ok(()));
    hold.abort();
}

#[tokio::test]
async fn inv_proxy_10_a_hung_origin_frees_its_slot_after_the_timeout() {
    let mut config = config(2);
    config.policy = config.policy.with_timeout(Duration::from_millis(300));
    let rig = rig_with(config).await;
    rig.fetcher.route("https://o.test/hang.mp4", Script::ok(pattern(10, 0)).hanging());
    rig.fetcher.route("https://o.test/after.mp4", Script::ok(pattern(10, 2)));
    let hung = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/hang.mp4", &HashMap::new(), Priority::Prefetch).await } });
    assert!(eventually(|| rig.fetcher.calls_for("https://o.test/hang.mp4") == 1).await);
    let queued = tokio::spawn({ let proxy = rig.proxy.clone(); async move { proxy.fetch_into_store("https://o.test/after.mp4", &HashMap::new(), Priority::Prefetch).await } });

    assert!(matches!(within(2, hung).await.unwrap(), Err(ProxyError::Fetch(_))));
    assert_eq!(within(2, queued).await.unwrap(), Ok(()));
    assert_eq!(rig.store.bytes("https://o.test/after.mp4"), Some(pattern(10, 2)));
}

#[tokio::test]
async fn inv_proxy_03_a_prefetched_block_loads_alone_and_serves_later_ranges_without_the_origin() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(4500, 7);
    let url = "https://o.test/blocks.mp4";
    rig.fetcher.route(url, Script::ok(data.clone()).chunked(300));
    let none = HashMap::new();

    rig.proxy.fetch_first_block(url, &none, Priority::Prefetch).await.unwrap();
    assert_eq!(fetched_ranges(&rig), vec![(0, Some(999))]);
    assert_eq!(rig.fetcher.bytes_served(), 1000);
    assert_eq!(rig.store.bytes("https://o.test/blocks.mp4#0"), Some(data[..1000].to_vec()));
    assert_eq!(rig.store.meta("https://o.test/blocks.mp4#0").await.unwrap().total_length, Some(4500));

    rig.proxy.fetch_first_block(url, &none, Priority::Prefetch).await.unwrap();
    rig.proxy.fetch_block_into_store(url, 2, &none, Priority::Prefetch).await.unwrap();
    assert_eq!(fetched_ranges(&rig), vec![(0, Some(999)), (2000, Some(2999))]);

    let calls = rig.fetcher.calls();
    let target = path_of(&rig.proxy.url_for(url, &none).unwrap());
    let ranged = request(rig.proxy.port(), "GET", &target, &[("Range", "bytes=2100-2199")]).await;
    assert_eq!(ranged.body, data[2100..2200]);
    assert_eq!(rig.fetcher.calls(), calls);
}

#[tokio::test]
async fn inv_proxy_03_a_small_range_from_zero_is_served_from_the_prefetched_first_block() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(4500, 7);
    let url = "https://o.test/first.mp4";
    rig.fetcher.route(url, Script::ok(data.clone()).chunked(300));
    let none = HashMap::new();
    rig.proxy.fetch_first_block(url, &none, Priority::Prefetch).await.unwrap();
    let calls = rig.fetcher.calls();

    let target = path_of(&rig.proxy.url_for(url, &none).unwrap());
    let reply = request(rig.proxy.port(), "GET", &target, &[("Range", "bytes=0-99")]).await;

    assert_eq!((reply.status, reply.header("content-range")), (206, Some("bytes 0-99/4500")));
    assert_eq!(reply.body, data[..100]);
    assert_eq!(rig.fetcher.calls(), calls);
}

#[tokio::test]
async fn inv_proxy_12_a_master_playlist_without_endlist_survives_a_restart_and_plays_offline() {
    let store = MemoryStore::new();
    let master = "https://o.test/live/master.m3u8";
    let first_fetcher = Arc::new(MockFetcher::new());
    first_fetcher.route(master, Script::ok(b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1000\nhttps://o.test/live/media.m3u8\n".to_vec()));
    let mut short_ttl = config(8);
    short_ttl.playlist_ttl = Duration::from_millis(50);
    let first = Proxy::start(short_ttl.clone(), first_fetcher.clone(), Arc::new(store.clone())).await.unwrap();
    let online = get(first.port(), &path_of(&first.url_for(master, &HashMap::new()).unwrap())).await;
    first.stop().await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    let offline = Arc::new(MockFetcher::new());
    offline.set_offline(true);
    let second = Proxy::start(short_ttl, offline.clone(), Arc::new(store)).await.unwrap();
    let replay = get(second.port(), &path_of(&second.url_for(master, &HashMap::new()).unwrap())).await;
    assert_eq!((replay.status, replay.body), (200, online.body));
    assert_eq!(offline.calls(), 0);
}

#[tokio::test]
async fn inv_proxy_02_a_store_failure_at_any_step_never_cuts_playback_for_any_joined_client() {
    for failure in [Failure::Begin, Failure::Append(1), Failure::Append(4), Failure::Commit] {
        let rig = rig().await;
        rig.store.fail_at(Some(failure));
        rig.fetcher.route(
            "https://o.test/big.mp4",
            Script::ok(pattern(5000, 9)).chunked(500).chunk_delay(Duration::from_millis(5)),
        );
        let target = path_of(&rig.proxy.url_for("https://o.test/big.mp4", &HashMap::new()).unwrap());
        let port = rig.proxy.port();
        let clients: Vec<_> = (0..3)
            .map(|_| tokio::spawn({ let target = target.clone(); async move { get(port, &target).await } }))
            .collect();
        for client in clients {
            let reply = within(10, client).await.unwrap();
            assert_eq!((reply.status, reply.body.len()), (200, 5000), "{failure:?}");
            assert_eq!(reply.body, pattern(5000, 9), "{failure:?}");
        }
        assert!(eventually(|| rig.store.open_writers() == 0 && rig.store.count() == 0).await, "{failure:?}");
        assert_eq!(rig.proxy.live_tasks(), 0, "{failure:?}");
    }
}

#[tokio::test]
async fn inv_proxy_02_a_store_failure_inside_a_block_never_cuts_a_range() {
    for failure in [Failure::Begin, Failure::Append(2), Failure::Commit] {
        let rig = rig_with(block_config(1000)).await;
        rig.store.fail_at(Some(failure));
        let data = pattern(4500, 5);
        rig.fetcher.route("https://o.test/r.mp4", Script::ok(data.clone()).chunked(250));
        let target = path_of(&rig.proxy.url_for("https://o.test/r.mp4", &HashMap::new()).unwrap());
        let reply = request(rig.proxy.port(), "GET", &target, &[("Range", "bytes=1500-3499")]).await;
        assert_eq!((reply.status, reply.header("content-range")), (206, Some("bytes 1500-3499/4500")), "{failure:?}");
        assert_eq!(reply.body, data[1500..3500], "{failure:?}");
    }
}

#[tokio::test]
async fn inv_proxy_01_registered_headers_reach_the_streams_own_urls_and_no_other_url_of_the_host() {
    let rig = rig().await;
    hls_origin(&rig);
    rig.fetcher.route("https://o.test/hls/other.mp4", Script::ok(pattern(10, 0)));
    let token = |value: &str| HashMap::from([("Authorization".to_string(), value.to_string())]);
    let media = rig.proxy.url_for(MEDIA, &token("token-stream")).unwrap();
    let other = rig.proxy.url_for("https://o.test/hls/other.mp4", &token("token-other")).unwrap();
    let port = rig.proxy.port();

    let playlist = get(port, &path_of(&media)).await;
    for line in media_lines(&playlist) {
        get(port, &follow(&media, &line)).await;
    }
    get(port, &path_of(&other)).await;

    let seen = |url: &str| {
        let request = rig.fetcher.requests().into_iter().find(|request| request.url == url).unwrap();
        request.headers.get("Authorization").cloned()
    };
    assert_eq!(seen(SEGMENT_ABSOLUTE).as_deref(), Some("token-stream"));
    assert_eq!(seen(SEGMENT_RELATIVE).as_deref(), Some("token-stream"));
    assert_eq!(seen("https://o.test/hls/other.mp4").as_deref(), Some("token-other"));
}

#[tokio::test]
async fn inv_proxy_06_abandoning_a_block_prefetch_cancels_its_download() {
    let rig = rig_with(block_config(1000)).await;
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(pattern(4500, 1)).chunked(50).chunk_delay(Duration::from_millis(10)));
    let owner = tokio::spawn({
        let proxy = rig.proxy.clone();
        async move { proxy.fetch_first_block("https://o.test/r.mp4", &HashMap::new(), Priority::Prefetch).await }
    });
    assert!(eventually(|| rig.store.open_writers() == 1).await);
    owner.abort();
    assert!(eventually(|| rig.proxy.live_tasks() == 0).await);
    assert_eq!((rig.store.open_writers(), rig.store.count(), rig.store.aborts()), (0, 0, 1));
}

#[tokio::test]
async fn inv_proxy_06_a_range_dropped_mid_block_lets_the_block_finish_for_the_next_range() {
    let rig = rig_with(block_config(1000)).await;
    let data = pattern(4500, 2);
    rig.fetcher.route("https://o.test/r.mp4", Script::ok(data.clone()).chunked(100).chunk_delay(Duration::from_millis(10)));
    let target = path_of(&rig.proxy.url_for("https://o.test/r.mp4", &HashMap::new()).unwrap());
    let mut stream = open(rig.proxy.port(), "GET", &target, &[("Range", "bytes=1100-1299")]).await;
    read_at_least(&mut stream, 300).await;
    drop(stream);
    assert!(eventually(|| rig.store.bytes("https://o.test/r.mp4#1").is_some()).await);
    assert_eq!(rig.store.bytes("https://o.test/r.mp4#1"), Some(data[1000..2000].to_vec()));
    assert!(eventually(|| rig.proxy.live_tasks() == 0).await);
}

#[tokio::test]
async fn inv_proxy_07_dropping_the_last_proxy_closes_the_listener_connections_and_downloads() {
    let rig = rig().await;
    rig.fetcher.route(
        "https://o.test/slow.mp4",
        Script::filled(10 * 1024 * 1024).chunked(16 * 1024).chunk_delay(Duration::from_millis(5)),
    );
    let target = path_of(&rig.proxy.url_for("https://o.test/slow.mp4", &HashMap::new()).unwrap());
    let port = rig.proxy.port();
    let mut stream = open(port, "GET", &target, &[]).await;
    read_at_least(&mut stream, 32 * 1024).await;
    let Rig { proxy, store, .. } = rig;
    let clone = proxy.clone();
    drop(proxy);
    assert_eq!(store.open_writers(), 1);
    drop(clone);

    assert!(eventually(|| store.open_writers() == 0 && store.count() == 0).await);
    assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err());
}

const IGNORING: &str = "https://o.test/voice.m4a";

async fn ignoring_rig(data: &[u8]) -> (Rig, String) {
    let rig = rig_with(block_config(1000)).await;
    rig.fetcher.route(IGNORING, Script::ok(data.to_vec()).chunked(300).chunk_delay(Duration::from_millis(5)).ignoring_range());
    let target = path_of(&rig.proxy.url_for(IGNORING, &HashMap::new()).unwrap());
    (rig, target)
}

#[tokio::test]
async fn inv_proxy_11_an_origin_that_answers_200_to_a_range_still_serves_every_range_shape_from_one_download() {
    let data = pattern(4500, 6);
    let cases: [(&str, u64, u64); 4] =
        [("bytes=0-1", 0, 1), ("bytes=2-", 2, 4499), ("bytes=-300", 4200, 4499), ("bytes=1100-1199", 1100, 1199)];
    for (range, start, end) in cases {
        let (rig, target) = ignoring_rig(&data).await;
        let port = rig.proxy.port();
        let reply = request(port, "GET", &target, &[("Range", range)]).await;
        let expected = format!("bytes {start}-{end}/4500");
        assert_eq!((reply.status, reply.header("content-range")), (206, Some(expected.as_str())), "{range}");
        assert_eq!(reply.header("content-length"), Some((end - start + 1).to_string().as_str()), "{range}");
        assert_eq!(reply.header("accept-ranges"), Some("bytes"), "{range}");
        assert_eq!(reply.body, data[start as usize..=end as usize], "{range}");
        let second = request(port, "GET", &target, &[("Range", "bytes=1000-1099")]).await;
        assert_eq!((second.status, second.body), (206, data[1000..1100].to_vec()), "{range}");
        assert!(eventually(|| rig.store.bytes(IGNORING) == Some(data.clone())).await, "{range}");
        let calls = rig.fetcher.calls();

        let third = request(port, "GET", &target, &[("Range", "bytes=1000-1099")]).await;
        assert_eq!((third.status, third.body), (206, data[1000..1100].to_vec()), "{range}");
        assert_eq!(rig.fetcher.calls(), calls, "{range}");
        assert_eq!(rig.store.count(), 1, "{range}");
    }
}

#[tokio::test]
async fn inv_proxy_05_parallel_ranges_of_an_origin_that_ignores_ranges_fetch_the_object_once() {
    let data = pattern(4500, 7);
    let (rig, target) = ignoring_rig(&data).await;
    let port = rig.proxy.port();
    let clients: Vec<_> = ["bytes=1100-1900", "bytes=3000-3400", "bytes=2-"]
        .into_iter()
        .map(|range| tokio::spawn({ let target = target.clone(); async move { (range, request(port, "GET", &target, &[("Range", range)]).await) } }))
        .collect();
    for client in clients {
        let (range, reply) = within(10, client).await.unwrap();
        assert_eq!(reply.status, 206, "{range}");
        let (start, end) = match range {
            "bytes=1100-1900" => (1100, 1900),
            "bytes=3000-3400" => (3000, 3400),
            _ => (2, 4499),
        };
        assert_eq!(reply.body, data[start..=end], "{range}");
    }
    assert!(eventually(|| rig.store.open_writers() == 0).await);
    assert_eq!(rig.fetcher.bytes_served(), data.len() as u64);
}

#[tokio::test]
async fn inv_proxy_21_an_object_above_the_response_limit_streams_when_the_origin_ignores_the_range() {
    let rig = rig_with(block_config(1000)).await;
    let size = crate::memory::InputLimits::default().response_bytes + 1;
    rig.fetcher.route(IGNORING, Script::filled(size).ignoring_range());
    let target = path_of(&rig.proxy.url_for(IGNORING, &HashMap::new()).unwrap());
    let reply = request(rig.proxy.port(), "GET", &target, &[("Range", "bytes=2-")]).await;
    assert_eq!((reply.status, reply.body.len() as u64), (206, size - 2));
    assert_eq!(rig.store.open_writers(), 0);
}

#[tokio::test]
async fn inv_proxy_21_a_range_of_an_origin_that_ignores_ranges_starts_answering_before_the_download_ends() {
    let data = pattern(6000, 3);
    let rig = rig_with(block_config(1000)).await;
    rig.fetcher.route(IGNORING, Script::ok(data.clone()).chunked(300).chunk_delay(Duration::from_millis(20)).ignoring_range());
    let target = path_of(&rig.proxy.url_for(IGNORING, &HashMap::new()).unwrap());
    let port = rig.proxy.port();
    let first = request(port, "GET", &target, &[("Range", "bytes=0-1")]).await;
    assert_eq!((first.status, first.body), (206, data[..2].to_vec()));

    let mut stream = open(port, "GET", &target, &[("Range", "bytes=2-")]).await;
    read_at_least(&mut stream, 1).await;
    let served = rig.fetcher.bytes_served();
    assert!(served < data.len() as u64 / 2, "the answer waited for {served} of {} origin bytes", data.len());
    let mut rest = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut rest).await.unwrap();
    assert!(rest.ends_with(&data[2..]));
}
