use super::memory_store::MemoryStore;
use super::proxy_tests::follow;
use super::test_client::{get, Reply};
use super::*;
use crate::fetch::mock::{MockFetcher, Script};
use futures_util::future::join_all;
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

const MEDIA: &str = "https://o.test/clip/media.m3u8";
const SEGMENTS: usize = 8;
const SEGMENT_BYTES: usize = 1_200;
const CHUNKS_AHEAD: usize = 3;
const SEGMENT_MS: u64 = 3_000;
const TARGET_MS: u64 = 4_000;

fn segment_url(index: usize) -> String {
    format!("https://cdn.test/clip/s{index}.ts")
}

fn clip() -> Vec<u8> {
    let mut text = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n");
    for index in 0..SEGMENTS {
        text.push_str(&format!("#EXTINF:3.000,\n{}\n", segment_url(index)));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    text.into_bytes()
}

struct FakePlayhead {
    position: AtomicU64,
    delay: Mutex<Duration>,
    hints: Mutex<Vec<u64>>,
}

impl FakePlayhead {
    fn new() -> Arc<FakePlayhead> {
        Arc::new(FakePlayhead { position: AtomicU64::new(0), delay: Mutex::new(Duration::ZERO), hints: Mutex::new(Vec::new()) })
    }

    fn move_to(&self, position_ms: u64) {
        self.position.store(position_ms, Ordering::SeqCst);
    }

    fn hints(&self) -> Vec<u64> {
        self.hints.lock().unwrap().clone()
    }
}

impl Playhead for FakePlayhead {
    fn position_ms(&self, _playlist: &str) -> PositionProbe {
        let delay = *self.delay.lock().unwrap();
        let position = self.position.load(Ordering::SeqCst);
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            Some(position)
        })
    }

    fn readahead(&self, _playlist: &str, hint_ms: u64) {
        self.hints.lock().unwrap().push(hint_ms);
    }
}

struct Rig {
    proxy: Proxy,
    fetcher: Arc<MockFetcher>,
    playhead: Arc<FakePlayhead>,
}

async fn rig() -> Rig {
    let mut config = Config::default();
    config.hold_ceiling = Duration::from_millis(1_500);
    config.probe_limit = Duration::from_millis(150);
    rig_with(config, true).await
}

async fn rig_with(config: Config, with_playhead: bool) -> Rig {
    let fetcher = Arc::new(MockFetcher::new());
    fetcher.route(MEDIA, Script::ok(clip()));
    for index in 0..SEGMENTS {
        fetcher.route(&segment_url(index), Script::ok(vec![index as u8; SEGMENT_BYTES]));
    }
    let proxy = Proxy::start(config, fetcher.clone(), Arc::new(MemoryStore::new())).await.expect("proxy start");
    let playhead = FakePlayhead::new();
    if with_playhead {
        proxy.set_playhead(playhead.clone());
    }
    Rig { proxy, fetcher, playhead }
}

async fn playlist_lines(rig: &Rig) -> Vec<String> {
    let port = rig.proxy.port();
    let url = rig.proxy.url_for(MEDIA, &HashMap::new()).expect("proxy url");
    let reply = get(port, &follow(&url, &url)).await;
    assert_eq!(reply.status, 200);
    reply.text().lines().filter(|line| !line.starts_with('#') && !line.is_empty()).map(|line| follow(&url, line)).collect()
}

fn ask(rig: &Rig, target: &str) -> JoinHandle<Reply> {
    let (port, target) = (rig.proxy.port(), target.to_string());
    tokio::spawn(async move { get(port, &target).await })
}

fn origin_calls(rig: &Rig, index: usize) -> usize {
    rig.fetcher.calls_for(&segment_url(index))
}

fn origin_segments(rig: &Rig) -> usize {
    (0..SEGMENTS).filter(|index| origin_calls(rig, *index) > 0).count()
}

async fn settle(milliseconds: u64) {
    tokio::time::sleep(Duration::from_millis(milliseconds)).await;
}

#[tokio::test]
async fn inv_readahead_01_a_player_that_asks_for_the_whole_clip_pulls_chunks_ahead_plus_one_from_the_origin() {
    let rig = rig().await;
    let port = rig.proxy.port();
    let targets = playlist_lines(&rig).await;
    let asks = join_all(targets.iter().map(|target| tokio::time::timeout(Duration::from_millis(400), get(port, target)))).await;
    assert_eq!(targets.len(), SEGMENTS);
    let served = asks.iter().filter(|ask| matches!(ask, Ok(reply) if reply.status == 200)).count();
    assert!(served >= 1, "no segment was served: {targets:?}");
    assert!(origin_segments(&rig) <= CHUNKS_AHEAD + 1, "origin fetched {} of {SEGMENTS} segments", origin_segments(&rig));
}

#[tokio::test]
async fn inv_readahead_02_a_held_segment_is_released_by_the_playhead_and_a_seek_and_a_cached_one_is_never_held() {
    let rig = rig().await;
    let targets = playlist_lines(&rig).await;
    let held = ask(&rig, &targets[5]);
    settle(250).await;
    assert_eq!((held.is_finished(), origin_calls(&rig, 5)), (false, 0));
    rig.playhead.move_to(2 * SEGMENT_MS);
    let reply = tokio::time::timeout(Duration::from_millis(1_400), held).await.expect("released by the playhead").unwrap();
    assert_eq!((reply.status, origin_calls(&rig, 5)), (200, 1));

    let jumped = ask(&rig, &targets[7]);
    settle(250).await;
    assert_eq!(jumped.is_finished(), false);
    rig.playhead.move_to(6 * SEGMENT_MS);
    rig.proxy.nudge_window(MEDIA);
    let reply = tokio::time::timeout(Duration::from_millis(300), jumped).await.expect("released by the seek").unwrap();
    assert_eq!(reply.status, 200);

    rig.playhead.move_to(0);
    rig.proxy.fetch_into_store(&segment_url(6), &HashMap::new(), Priority::Prefetch).await.expect("prefetched");
    let started = Instant::now();
    let cached = get(rig.proxy.port(), &targets[6]).await;
    assert_eq!((cached.status, origin_calls(&rig, 6)), (200, 1));
    assert!(started.elapsed() < Duration::from_millis(250), "a stored segment was held for {:?}", started.elapsed());
}

#[tokio::test]
async fn inv_readahead_03_the_playing_item_is_never_blocked_by_an_unknown_or_late_playhead() {
    let rig = rig().await;
    let targets = playlist_lines(&rig).await;
    *rig.playhead.delay.lock().unwrap() = Duration::from_secs(2);
    let started = Instant::now();
    let late = get(rig.proxy.port(), &targets[6]).await;
    assert_eq!(late.status, 200);
    assert!(started.elapsed() < Duration::from_millis(600), "a late playhead held the request for {:?}", started.elapsed());

    let bare = rig_with(Config::default(), false).await;
    let targets = playlist_lines(&bare).await;
    let started = Instant::now();
    assert_eq!(get(bare.proxy.port(), &targets[7]).await.status, 200);
    assert!(started.elapsed() < Duration::from_millis(250), "no playhead held the request for {:?}", started.elapsed());

    *rig.playhead.delay.lock().unwrap() = Duration::ZERO;
    let started = Instant::now();
    assert_eq!(get(rig.proxy.port(), &targets[7]).await.status, 200);
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(1_400) && waited < Duration::from_millis(2_500), "the ceiling held the request for {waited:?}");
}

#[tokio::test]
async fn inv_readahead_04_a_stall_grows_the_cap_by_one_chunk_up_to_eight_and_a_new_playlist_starts_over() {
    let rig = rig().await;
    let targets = playlist_lines(&rig).await;
    let held = ask(&rig, &targets[4]);
    settle(250).await;
    assert_eq!(held.is_finished(), false);
    rig.proxy.grow_window(MEDIA);
    let reply = tokio::time::timeout(Duration::from_millis(300), held).await.expect("released by the growth").unwrap();
    assert_eq!(reply.status, 200);
    for _ in 0..10 {
        rig.proxy.grow_window(MEDIA);
    }
    assert_eq!(rig.playhead.hints().last().copied(), Some(8 * TARGET_MS));
    playlist_lines(&rig).await;
    assert_eq!(rig.playhead.hints().last().copied(), Some(CHUNKS_AHEAD as u64 * TARGET_MS));
}

#[tokio::test]
async fn inv_readahead_06_the_proxy_tells_the_player_side_how_far_ahead_to_buffer_when_it_serves_a_playlist() {
    let rig = rig().await;
    playlist_lines(&rig).await;
    assert_eq!(rig.playhead.hints(), vec![CHUNKS_AHEAD as u64 * TARGET_MS]);
}

#[tokio::test]
async fn inv_readahead_07_a_memory_step_lowers_the_cap_and_the_hint_down_to_one_chunk_and_back() {
    let rig = rig().await;
    let targets = playlist_lines(&rig).await;
    rig.proxy.set_readahead_percent(25);
    assert_eq!(rig.playhead.hints().last().copied(), Some(TARGET_MS));
    let held = ask(&rig, &targets[2]);
    settle(250).await;
    assert_eq!((held.is_finished(), origin_calls(&rig, 2)), (false, 0));
    rig.proxy.set_readahead_percent(100);
    let reply = tokio::time::timeout(Duration::from_millis(300), held).await.expect("released by the full cap").unwrap();
    assert_eq!(reply.status, 200);
}
