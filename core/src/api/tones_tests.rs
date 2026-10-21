use super::tones::{CacheTones, Tones};
use super::*;
use crate::analysis::{analyze, ToneSource};
use crate::cache::{Cache, Kind as CacheKind};
use crate::limits::Limits;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;

const KNOWN: &str = "LEHV6nWB2yk8pyo0adR*.7kCMdnj";
const WAIT_LIMIT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn inv_tone_13_pixels_without_demand_do_no_work() {
    let rig = rig(8).await;
    let source = item("https://cdn.test/unwatched.jpg", None);
    offer(&rig.tones, source, 90);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.tones.analyses(), 0, "pixel analyses without tone demand");
}

struct Rig {
    cache: Cache,
    tones: Arc<Tones>,
    _dir: tempfile::TempDir,
}

async fn rig(memo: usize) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), Limits::default(), u64::MAX);
    cache.ready().await;
    let tones = Tones::new(Handle::current(), Arc::new(CacheTones::new(cache.clone(), Arc::new(|url, _| url.to_string()))), memo);
    Rig { cache, tones, _dir: dir }
}

fn item(url: &str, blur_hash: Option<&str>) -> MediaItem {
    MediaItem { blur_hash: blur_hash.map(str::to_string), ..MediaItem::bare(url, Kind::Image) }
}

fn pixels(level: u8) -> Vec<u8> {
    [[level, level, level, 255]; 16].concat()
}

struct Prepared(u8);

#[async_trait::async_trait]
impl TonePreparation for Prepared {
    async fn prepare(&self, _source: MediaItem) -> Option<TonePixels> {
        Some(TonePixels { width: 4, height: 4, rgba: pixels(self.0) })
    }
}

fn offer(tones: &Arc<Tones>, source: MediaItem, level: u8) {
    tones.set_preparation(Arc::new(Prepared(level)));
    tones.refresh(&source.url);
}

struct Counted {
    calls: AtomicUsize,
    bytes: AtomicUsize,
    started: tokio::sync::Notify,
    release: tokio::sync::Semaphore,
}

impl Counted {
    fn new() -> Arc<Self> {
        Arc::new(Self { calls: AtomicUsize::new(0), bytes: AtomicUsize::new(0), started: tokio::sync::Notify::new(), release: tokio::sync::Semaphore::new(0) })
    }
}

#[async_trait::async_trait]
impl TonePreparation for Counted {
    async fn prepare(&self, _source: MediaItem) -> Option<TonePixels> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        self.release.acquire().await.unwrap().forget();
        let rgba = pixels(90);
        self.bytes.fetch_add(rgba.len(), Ordering::SeqCst);
        Some(TonePixels { width: 4, height: 4, rgba })
    }
}

#[tokio::test]
async fn inv_tone_13_late_demand_prepares_once_and_cached_pixels_skip_preparation() {
    let rig = rig(8).await;
    let source = item("https://cdn.test/late.jpg", Some(KNOWN));
    let preparation = Counted::new();
    rig.tones.set_preparation(preparation.clone());
    assert!(!rig.tones.pixels_needed(&source).await);
    assert_eq!((preparation.calls.load(Ordering::SeqCst), preparation.bytes.load(Ordering::SeqCst), rig.tones.analyses()), (0, 0, 0));
    let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
    let listener = rig.tones.subscribe(&source, Arc::new(move |value| { sent.send(value).unwrap(); }));
    tokio::time::timeout(WAIT_LIMIT, preparation.started.notified()).await.unwrap();
    for _ in 0..8 { rig.tones.refresh(&source.url); }
    tokio::task::yield_now().await;
    assert_eq!(preparation.calls.load(Ordering::SeqCst), 1);
    preparation.release.add_permits(1);
    assert_eq!(tokio::time::timeout(WAIT_LIMIT, received.recv()).await.unwrap(), Some(ToneSource::Pixels));
    assert!(!rig.tones.pixels_needed(&source).await);
    rig.tones.refresh(&source.url);
    assert_eq!((preparation.calls.load(Ordering::SeqCst), preparation.bytes.load(Ordering::SeqCst), rig.tones.analyses()), (1, 64, 1));
    drop(listener);
}

#[tokio::test]
async fn inv_tone_13_last_listener_cancels_preparation_and_later_demand_retries() {
    let rig = rig(8).await;
    let source = item("https://cdn.test/cancel.jpg", None);
    let preparation = Counted::new();
    rig.tones.set_preparation(preparation.clone());
    let listener = rig.tones.subscribe(&source, Arc::new(|_| {}));
    tokio::time::timeout(WAIT_LIMIT, preparation.started.notified()).await.unwrap();
    drop(listener);
    let next = rig.tones.subscribe(&source, Arc::new(|_| {}));
    tokio::time::timeout(WAIT_LIMIT, preparation.started.notified()).await.unwrap();
    assert_eq!(preparation.calls.load(Ordering::SeqCst), 2);
    assert_eq!(preparation.bytes.load(Ordering::SeqCst), 0);
    preparation.release.add_permits(1);
    wait_for_source(&rig.tones, &source, ToneSource::Pixels).await;
    assert_eq!(rig.tones.analyses(), 1);
    drop(next);
}

struct FailsOnce {
    tones: std::sync::Weak<Tones>,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl TonePreparation for FailsOnce {
    async fn prepare(&self, source: MediaItem) -> Option<TonePixels> {
        assert!(self.tones.upgrade().unwrap().pixels_needed(&source).await);
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 { return None; }
        Some(TonePixels { width: 4, height: 4, rgba: pixels(80) })
    }
}

#[tokio::test]
async fn inv_tone_13_failed_preparation_releases_request_and_callback_can_reenter_core() {
    let rig = rig(8).await;
    let source = item("https://cdn.test/retry.jpg", None);
    let preparation = Arc::new(FailsOnce { tones: Arc::downgrade(&rig.tones), calls: AtomicUsize::new(0) });
    rig.tones.set_preparation(preparation.clone());
    assert_eq!(rig.tones.tone(&source, 0.15).await, Ok(None));
    wait_for(|| preparation.calls.load(Ordering::SeqCst) == 1).await;
    assert_eq!(rig.tones.analyses(), 0);
    assert_eq!(rig.tones.tone(&source, 0.15).await, Ok(None));
    wait_for_source(&rig.tones, &source, ToneSource::Pixels).await;
    assert_eq!(preparation.calls.load(Ordering::SeqCst), 2);
    assert_eq!(rig.tones.analyses(), 1);
}

async fn wait_for(mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < WAIT_LIMIT, "condition not reached in time");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn wait_for_source(tones: &Arc<Tones>, source: &MediaItem, wanted: ToneSource) {
    let started = Instant::now();
    loop {
        if tones.tone(source, 0.15).await.unwrap().is_some_and(|tone| tone.source == wanted) {
            return;
        }
        assert!(started.elapsed() < WAIT_LIMIT, "tone from {wanted:?} did not arrive");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn inv_tone_09_a_query_answers_not_ready_then_the_blurhash_then_the_pixels_replace_it() {
    let rig = rig(8).await;
    let plain = item("https://cdn.test/plain.jpg", None);
    let hashed = item("https://cdn.test/hashed.jpg", Some(KNOWN));
    rig.cache.put(&hashed.url, CacheKind::Image, b"bytes").await.unwrap();

    assert_eq!(rig.tones.tone(&plain, 0.15).await, Ok(None));
    assert_eq!(rig.tones.tone(&item("https://cdn.test/bad.jpg", Some("short")), 0.15).await, Ok(None));
    let before = rig.tones.tone(&hashed, 0.15).await.unwrap().unwrap();
    assert_eq!(before.source, ToneSource::BlurHash);
    assert_eq!(rig.cache.tone(&hashed.url), None);

    offer(&rig.tones, hashed.clone(), 255);
    wait_for_source(&rig.tones, &hashed, ToneSource::Pixels).await;

    let after = rig.tones.tone(&hashed, 0.15).await.unwrap().unwrap();
    assert_eq!(after.top.luminance, 1.0);
    assert_ne!(after.top, before.top);
    assert_eq!(rig.cache.tone(&hashed.url), Some(analyze(4, 4, &pixels(255), ToneSource::Pixels).unwrap()));
    assert!(rig.tones.tone(&hashed, 0.0).await.is_err());
}

#[tokio::test]
async fn inv_tone_09_a_source_without_a_cache_entry_keeps_its_tone_in_a_bounded_memory_table() {
    let rig = rig(2).await;
    let local = |name: &str| item(&format!("file:///tmp/{name}.png"), None);

    for name in ["a", "b", "c"] {
        offer(&rig.tones, local(name), 120);
        wait_for_source(&rig.tones, &local(name), ToneSource::Pixels).await;
    }

    assert_eq!(rig.tones.tone(&local("a"), 0.15).await, Ok(None));
    assert!(rig.tones.tone(&local("b"), 0.15).await.unwrap().is_some());
    assert!(rig.tones.tone(&local("c"), 0.15).await.unwrap().is_some());
}

#[tokio::test]
async fn inv_tone_10_the_analysis_runs_once_per_key_however_many_times_the_pixels_arrive() {
    let rig = rig(8).await;
    let first = item("https://cdn.test/one.jpg", None);
    let second = item("https://cdn.test/two.jpg", None);
    rig.cache.put(&first.url, CacheKind::Image, b"one").await.unwrap();
    rig.cache.put(&second.url, CacheKind::Image, b"two").await.unwrap();

    for _ in 0..8 {
        offer(&rig.tones, first.clone(), 90);
    }
    wait_for_source(&rig.tones, &first, ToneSource::Pixels).await;
    for _ in 0..8 {
        offer(&rig.tones, first.clone(), 90);
    }
    offer(&rig.tones, second.clone(), 90);
    wait_for_source(&rig.tones, &second, ToneSource::Pixels).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(rig.tones.analyses(), 2);
    rig.cache.flush().await.unwrap();
    let again = Tones::new(Handle::current(), Arc::new(CacheTones::new(rig.cache.clone(), Arc::new(|url, _| url.to_string()))), 8);
    offer(&again, first.clone(), 90);
    again.tone(&first, 0.15).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(again.analyses(), 0);
}

#[tokio::test]
async fn inv_tone_11_a_listener_hears_every_stored_or_replaced_tone_of_its_source_and_nothing_after_it_leaves() {
    let rig = rig(8).await;
    let watched = item("https://cdn.test/watched.jpg", None);
    let other = item("https://cdn.test/other.jpg", None);
    for source in [&watched, &other] {
        rig.cache.put(&source.url, CacheKind::Image, b"bytes").await.unwrap();
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    let count = Arc::new(AtomicUsize::new(0));
    let listener = {
        let (heard, count) = (heard.clone(), count.clone());
        rig.tones.subscribe(&watched, Arc::new(move |source: ToneSource| {
            heard.lock().unwrap().push(source);
            count.fetch_add(1, Ordering::SeqCst);
        }))
    };

    offer(&rig.tones, other.clone(), 10);
    offer(&rig.tones, watched.clone(), 10);
    wait_for(|| count.load(Ordering::SeqCst) == 1).await;
    rig.cache.remove(&watched.url).await;
    rig.cache.put(&watched.url, CacheKind::Image, b"replaced bytes").await.unwrap();
    offer(&rig.tones, watched.clone(), 240);
    wait_for(|| count.load(Ordering::SeqCst) == 2).await;

    assert_eq!(*heard.lock().unwrap(), vec![ToneSource::Pixels, ToneSource::Pixels]);
    drop(listener);
    rig.cache.remove(&watched.url).await;
    rig.cache.put(&watched.url, CacheKind::Image, b"third bytes").await.unwrap();
    offer(&rig.tones, watched.clone(), 60);
    wait_for_source(&rig.tones, &watched, ToneSource::Pixels).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(rig.tones.listener_count(), 0);
}

#[tokio::test]
async fn inv_tone_09_a_video_tone_rides_on_its_first_block_and_leaves_with_it() {
    let rig = rig(8).await;
    let video = MediaItem::bare("https://cdn.test/clip.mp4", Kind::Video);
    let block = crate::proxy::block_key(&video.url, 0);
    rig.cache.put(&block, CacheKind::Stream, b"first block").await.unwrap();

    offer(&rig.tones, video.clone(), 30);
    wait_for_source(&rig.tones, &video, ToneSource::Pixels).await;

    assert!(rig.cache.tone(&block).is_some());
    assert_eq!(rig.cache.tone(&video.url), None);
    rig.cache.purge_prefix(&format!("{}#", video.url)).await;
    assert_eq!(rig.tones.tone(&video, 0.15).await, Ok(None));
}
