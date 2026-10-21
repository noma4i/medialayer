use super::test_client::request;
use super::*;
use crate::cache::{classify_kind, Cache, CacheStore};
use crate::fetch::mock::{MockFetcher, Script};
use crate::limits::Limits;
use std::time::Duration;

const VIDEO: &str = "https://cdn.test/circle.mp4";
const SIZE: usize = 6_000_000;
const RANGES: [&str; 5] = ["bytes=0-1", "bytes=0-1048575", "bytes=-1000", "bytes=2097152-", "bytes=5000000-5999999"];

#[derive(Clone, Copy, Debug)]
enum Shape {
    Honest,
    HonestWithoutLength,
    Ignoring,
    IgnoringWithoutLength,
    IgnoringAfterTheProbe,
}

fn pattern() -> Vec<u8> {
    (0..SIZE).map(|index| (index % 251) as u8).collect()
}

fn script(shape: Shape) -> Script {
    let script = Script::ok(pattern()).chunked(16 * 1024);
    match shape {
        Shape::Honest => script,
        Shape::HonestWithoutLength => script.without_length(),
        Shape::Ignoring => script.ignoring_range(),
        Shape::IgnoringWithoutLength => script.ignoring_range().without_length(),
        Shape::IgnoringAfterTheProbe => script.ignoring_range_after(1),
    }
}

fn expected_span(range: &str) -> (usize, usize) {
    let (from, to) = range.trim_start_matches("bytes=").split_once('-').unwrap();
    match (from.parse::<usize>(), to.parse::<usize>()) {
        (Ok(from), Ok(to)) => (from, to),
        (Ok(from), Err(_)) => (from, SIZE - 1),
        (Err(_), Ok(suffix)) => (SIZE - suffix, SIZE - 1),
        _ => unreachable!(),
    }
}

async fn answer(shape: Shape, range: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), Limits { budget: 64 * 1024 * 1024, min_budget: 1, ..Limits::default() }, u64::MAX);
    cache.ready().await;
    let fetcher = Arc::new(MockFetcher::new());
    fetcher.route(VIDEO, script(shape));
    let mut config = Config::default();
    config.policy = config.policy.with_timeout(Duration::from_secs(5));
    let proxy = Proxy::start(config, fetcher, Arc::new(CacheStore::new(cache, Arc::new(classify_kind)))).await.unwrap();
    let routed = url::Url::parse(&proxy.url_for(VIDEO, &HashMap::new()).unwrap()).unwrap();
    let target = format!("{}?{}", routed.path(), routed.query().unwrap());
    let reply = tokio::time::timeout(Duration::from_secs(20), request(proxy.port(), "GET", &target, &[("Range", range)])).await;
    let Ok(reply) = reply else { return format!("{shape:?} {range}: no answer in 20 s") };
    let (first, last) = expected_span(range);
    let expected_range = format!("bytes {first}-{last}/{SIZE}");
    let right = reply.status == 206 && reply.header("content-range") == Some(expected_range.as_str()) && reply.body == pattern()[first..=last];
    if right { String::new() } else { format!("{shape:?} {range}: status={} bytes={}", reply.status, reply.body.len()) }
}

#[tokio::test]
async fn inv_proxy_22_every_range_shape_gets_206_with_the_right_bytes_from_every_origin_shape() {
    let shapes = [Shape::Honest, Shape::HonestWithoutLength, Shape::Ignoring, Shape::IgnoringWithoutLength, Shape::IgnoringAfterTheProbe];
    let mut broken = Vec::new();
    for shape in shapes {
        for range in RANGES {
            broken.push(answer(shape, range).await);
        }
    }
    broken.retain(|line| !line.is_empty());
    assert!(broken.is_empty(), "{broken:#?}");
}
