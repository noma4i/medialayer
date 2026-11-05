use super::memory_store::MemoryStore;
use super::test_client::Reply;
use super::*;
use crate::fetch::mock::{MockFetcher, Script};
use std::future::Future;
use std::time::Duration;
use url::Url;

pub(super) const MASTER: &str = "https://o.test/hls/master.m3u8";
pub(super) const MEDIA: &str = "https://o.test/hls/media.m3u8";
pub(super) const SEGMENT_ABSOLUTE: &str = "https://cdn.test/seg0.ts";
pub(super) const SEGMENT_RELATIVE: &str = "https://o.test/hls/seg1.ts";
pub(super) const PEAK_CHUNK_LIMIT: usize = 8;

pub(super) struct Rig {
    pub(super) proxy: Proxy,
    pub(super) fetcher: Arc<MockFetcher>,
    pub(super) store: MemoryStore,
}

pub(super) fn config(max_concurrent: usize) -> Config {
    let mut config = Config::default();
    config.max_concurrent = max_concurrent;
    config.policy = config.policy.with_timeout(Duration::from_secs(5));
    config
}

pub(super) async fn rig() -> Rig {
    rig_with(config(8)).await
}

pub(super) async fn rig_with(config: Config) -> Rig {
    let fetcher = Arc::new(MockFetcher::new());
    let store = MemoryStore::new();
    let proxy = Proxy::start(config, fetcher.clone(), Arc::new(store.clone())).await.expect("proxy start");
    Rig { proxy, fetcher, store }
}

pub(super) fn pattern(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|index| (index as u8).wrapping_add(seed)).collect()
}

pub(super) fn hls_origin(rig: &Rig) {
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

pub(super) fn path_of(proxy_url: &str) -> String {
    let parsed = Url::parse(proxy_url).unwrap();
    format!("{}?{}", parsed.path(), parsed.query().unwrap_or_default())
}

pub(super) fn follow(base: &str, line: &str) -> String {
    let resolved = Url::parse(base).unwrap().join(line).unwrap();
    match resolved.query() {
        Some(query) => format!("{}?{query}", resolved.path()),
        None => resolved.path().to_string(),
    }
}

pub(super) fn media_lines(reply: &Reply) -> Vec<String> {
    reply.text().lines().filter(|line| !line.starts_with('#') && !line.is_empty()).map(str::to_string).collect()
}

pub(super) async fn eventually(mut condition: impl FnMut() -> bool) -> bool {
    for _ in 0..300 {
        if condition() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    condition()
}

pub(super) async fn within<T>(seconds: u64, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(seconds), future).await.expect("timed out")
}

pub(super) fn fetched_ranges(rig: &Rig) -> Vec<(u64, Option<u64>)> {
    rig.fetcher.requests().iter().filter_map(|request| request.range).collect()
}

pub(super) fn block_config(block: u64) -> Config {
    let mut config = config(8);
    config.chunk_size = block;
    config
}
