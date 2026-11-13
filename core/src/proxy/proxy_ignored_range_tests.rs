use super::test_client::{get, open, read_at_least, request};
use super::*;
use crate::fetch::mock::Script;
use std::time::Duration;
use super::proxy_harness::*;

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

// mutation: a first-block load that gets 200 to its range fails with "origin ignored the range" instead of keeping the whole body
#[tokio::test]
async fn inv_proxy_11_a_first_block_load_from_an_origin_that_ignores_ranges_succeeds_and_serves_ranges() {
    let data = pattern(4500, 8);
    let (rig, target) = ignoring_rig(&data).await;

    let loaded = rig.proxy.fetch_first_block(IGNORING, &HashMap::new(), Priority::Playback).await;
    assert!(loaded.is_ok(), "{loaded:?}");

    let reply = request(rig.proxy.port(), "GET", &target, &[("Range", "bytes=0-99")]).await;
    assert_eq!((reply.status, reply.header("content-range")), (206, Some("bytes 0-99/4500")));
    assert_eq!(reply.body, data[0..100].to_vec());
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

#[tokio::test]
async fn inv_proxy_25_a_download_that_received_every_byte_is_committed_even_when_the_last_client_leaves_before_the_stream_ends() {
    let rig = rig().await;
    rig.fetcher.route("https://o.test/last.ts", Script::ok(pattern(4000, 5)).chunked(1000).end_delay(Duration::from_millis(400)));
    let target = path_of(&rig.proxy.url_for("https://o.test/last.ts", &HashMap::new()).unwrap());
    let port = rig.proxy.port();

    let mut stream = open(port, "GET", &target, &[]).await;
    assert!(read_at_least(&mut stream, 4000).await >= 4000);
    drop(stream);
    tokio::time::sleep(Duration::from_millis(150)).await;

    assert_eq!(rig.store.commits(), 1);
    let again = get(port, &target).await;
    assert_eq!((again.status, again.body), (200, pattern(4000, 5)));
    assert_eq!(rig.fetcher.calls_for("https://o.test/last.ts"), 1);
}

#[tokio::test]
async fn inv_proxy_26_a_player_request_does_not_wait_for_prefetch_flights_in_progress() {
    let rig = rig_with(config(2)).await;
    for index in 0..3 {
        rig.fetcher.route(&format!("https://o.test/p{index}.ts"), Script::ok(pattern(100, index)).head_delay(Duration::from_millis(400)));
    }
    for name in ["alone", "contended"] {
        rig.fetcher.route(&format!("https://o.test/{name}.ts"), Script::ok(pattern(100, 9)).head_delay(Duration::from_millis(50)));
    }
    let port = rig.proxy.port();
    let timed = |name: &'static str| {
        let target = path_of(&rig.proxy.url_for(&format!("https://o.test/{name}.ts"), &HashMap::new()).unwrap());
        async move {
            let started = std::time::Instant::now();
            let reply = get(port, &target).await;
            assert_eq!(reply.status, 200);
            started.elapsed()
        }
    };
    let alone = timed("alone").await;
    for index in 0..3 {
        let proxy = rig.proxy.clone();
        tokio::spawn(async move { proxy.fetch_into_store(&format!("https://o.test/p{index}.ts"), &HashMap::new(), Priority::Prefetch, None).await });
    }
    assert!(eventually(|| rig.fetcher.calls_for("https://o.test/p0.ts") == 1).await);

    let contended = timed("contended").await;

    assert!(contended < alone + Duration::from_millis(150), "player request took {contended:?} against {alone:?} alone");
}
