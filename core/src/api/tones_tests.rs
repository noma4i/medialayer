use super::tones::{CacheTones, Tones};
use super::*;
use crate::analysis::{analyze, MediaTone, ToneSource};
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
    let listener = rig.tones.subscribe(&source, 0.15, Arc::new(move |value: Option<MediaTone>| { sent.send(value).unwrap(); })).unwrap();
    tokio::time::timeout(WAIT_LIMIT, preparation.started.notified()).await.unwrap();
    for _ in 0..8 { rig.tones.refresh(&source.url); }
    tokio::task::yield_now().await;
    assert_eq!(preparation.calls.load(Ordering::SeqCst), 1);
    preparation.release.add_permits(1);
    let mut sources = Vec::new();
    while sources.last() != Some(&Some(ToneSource::Pixels)) {
        let value = tokio::time::timeout(WAIT_LIMIT, received.recv()).await.unwrap().unwrap();
        sources.push(value.map(|tone| tone.source));
    }
    assert!(sources[..sources.len() - 1].iter().all(|source| *source == Some(ToneSource::BlurHash)), "{sources:?}");
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
    let listener = rig.tones.subscribe(&source, 0.15, Arc::new(|_| {})).unwrap();
    tokio::time::timeout(WAIT_LIMIT, preparation.started.notified()).await.unwrap();
    drop(listener);
    let next = rig.tones.subscribe(&source, 0.15, Arc::new(|_| {})).unwrap();
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
        rig.tones.subscribe(&watched, 0.15, Arc::new(move |tone: Option<MediaTone>| {
            heard.lock().unwrap().push(tone.map(|tone| tone.source));
            count.fetch_add(1, Ordering::SeqCst);
        })).unwrap()
    };
    wait_for(|| count.load(Ordering::SeqCst) == 1).await;

    offer(&rig.tones, other.clone(), 10);
    offer(&rig.tones, watched.clone(), 10);
    wait_for(|| count.load(Ordering::SeqCst) == 2).await;
    rig.cache.remove(&watched.url).await;
    rig.cache.put(&watched.url, CacheKind::Image, b"replaced bytes").await.unwrap();
    offer(&rig.tones, watched.clone(), 240);
    wait_for(|| count.load(Ordering::SeqCst) == 3).await;

    assert_eq!(*heard.lock().unwrap(), vec![None, Some(ToneSource::Pixels), Some(ToneSource::Pixels)]);
    drop(listener);
    rig.cache.remove(&watched.url).await;
    rig.cache.put(&watched.url, CacheKind::Image, b"third bytes").await.unwrap();
    offer(&rig.tones, watched.clone(), 60);
    wait_for_source(&rig.tones, &watched, ToneSource::Pixels).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(count.load(Ordering::SeqCst), 3);
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

/// Holds the first `holds` reads of the tone store until a permit arrives and records how the reads overlap.
struct Gated {
    inner: CacheTones,
    holds: AtomicUsize,
    gate: tokio::sync::Semaphore,
    started: AtomicUsize,
    inflight: AtomicUsize,
    peak: AtomicUsize,
    dropped: AtomicUsize,
}

struct Reading<'a> {
    store: &'a Gated,
    finished: bool,
}

impl Drop for Reading<'_> {
    fn drop(&mut self) {
        self.store.inflight.fetch_sub(1, Ordering::SeqCst);
        if !self.finished {
            self.store.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[async_trait::async_trait]
impl ToneStore for Gated {
    fn key_for(&self, url: &str, headers: &std::collections::HashMap<String, String>) -> String {
        self.inner.key_for(url, headers)
    }

    async fn tone(&self, key: &str) -> Option<crate::analysis::ToneData> {
        self.started.fetch_add(1, Ordering::SeqCst);
        let now = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        let mut reading = Reading { store: self, finished: false };
        let tone = self.inner.tone(key).await;
        if self.holds.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(1)).is_ok() {
            self.gate.acquire().await.unwrap().forget();
        }
        reading.finished = true;
        tone
    }

    async fn set_tone(&self, key: &str, tone: crate::analysis::ToneData) -> bool {
        self.inner.set_tone(key, tone).await
    }
}

struct GatedRig {
    cache: Cache,
    tones: Arc<Tones>,
    store: Arc<Gated>,
    _dir: tempfile::TempDir,
}

async fn gated_rig(holds: usize) -> GatedRig {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), Limits::default(), u64::MAX);
    cache.ready().await;
    let inner = CacheTones::new(cache.clone(), Arc::new(|url, _| url.to_string()));
    let store = Arc::new(Gated {
        inner,
        holds: AtomicUsize::new(holds),
        gate: tokio::sync::Semaphore::new(0),
        started: AtomicUsize::new(0),
        inflight: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        dropped: AtomicUsize::new(0),
    });
    let tones = Tones::new(Handle::current(), store.clone(), 8);
    GatedRig { cache, tones, store, _dir: dir }
}

type Heard = Arc<Mutex<Vec<Option<MediaTone>>>>;

fn recording(heard: &Heard) -> Arc<dyn Fn(Option<MediaTone>) + Send + Sync> {
    let heard = heard.clone();
    Arc::new(move |tone| heard.lock().unwrap().push(tone))
}

#[tokio::test]
async fn inv_tone_15_a_change_between_subscribing_and_the_first_read_is_not_lost() {
    let rig = gated_rig(1).await;
    let source = item("https://cdn.test/raced.jpg", None);
    rig.cache.put(&source.url, CacheKind::Image, b"bytes").await.unwrap();
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));
    let _subscription = rig.tones.subscribe(&source, 0.15, recording(&heard)).unwrap();
    wait_for(|| rig.store.started.load(Ordering::SeqCst) == 1).await;

    offer(&rig.tones, source.clone(), 200);
    wait_for(|| rig.cache.tone(&source.url).is_some()).await;
    rig.store.gate.add_permits(1);

    wait_for(|| heard.lock().unwrap().last().is_some_and(|tone| tone.as_ref().is_some_and(|tone| tone.source == ToneSource::Pixels))).await;
    assert_eq!(heard.lock().unwrap().first(), Some(&None), "the first value is the state read before the change");
}

#[tokio::test]
async fn inv_tone_16_rapid_changes_keep_one_reader_and_one_pending_mark() {
    let rig = gated_rig(2).await;
    let source = item("https://cdn.test/rapid.jpg", None);
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));
    let _subscription = rig.tones.subscribe(&source, 0.15, recording(&heard)).unwrap();
    wait_for(|| rig.store.started.load(Ordering::SeqCst) == 1).await;
    for _ in 0..50 { rig.tones.changed(&source.url); }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.store.started.load(Ordering::SeqCst), 1, "a change started a read while one was held");

    rig.store.gate.add_permits(1);
    wait_for(|| rig.store.started.load(Ordering::SeqCst) == 2).await;
    for _ in 0..50 { rig.tones.changed(&source.url); }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(rig.store.started.load(Ordering::SeqCst), 2);

    rig.store.gate.add_permits(1);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(rig.store.started.load(Ordering::SeqCst) <= 4, "reads: {}", rig.store.started.load(Ordering::SeqCst));
    assert_eq!(rig.store.peak.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn inv_tone_17_cancel_drops_a_held_read_and_an_old_generation_never_delivers() {
    let rig = gated_rig(1).await;
    let old = item("https://cdn.test/old.jpg", None);
    let new = item("https://cdn.test/new.jpg", None);
    let heard_old: Heard = Arc::new(Mutex::new(Vec::new()));
    let heard_new: Heard = Arc::new(Mutex::new(Vec::new()));
    let first = rig.tones.subscribe(&old, 0.15, recording(&heard_old)).unwrap();
    wait_for(|| rig.store.started.load(Ordering::SeqCst) == 1).await;

    drop(first);
    wait_for(|| rig.store.dropped.load(Ordering::SeqCst) == 1).await;
    let _second = rig.tones.subscribe(&new, 0.15, recording(&heard_new)).unwrap();
    wait_for(|| !heard_new.lock().unwrap().is_empty()).await;
    rig.store.gate.add_permits(1);
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert!(heard_old.lock().unwrap().is_empty(), "the cancelled subscription delivered");
    assert_eq!(rig.tones.listener_count(), 1);
}

#[tokio::test]
async fn inv_tone_18_a_band_change_reads_the_stored_tone_without_a_new_analysis() {
    let rig = rig(8).await;
    let source = item("https://cdn.test/banded.jpg", None);
    rig.cache.put(&source.url, CacheKind::Image, b"bytes").await.unwrap();
    let preparation = Counted::new();
    preparation.release.add_permits(8);
    rig.tones.set_preparation(preparation.clone());
    let narrow: Heard = Arc::new(Mutex::new(Vec::new()));
    let first = rig.tones.subscribe(&source, 0.15, recording(&narrow)).unwrap();
    wait_for(|| narrow.lock().unwrap().iter().any(|tone| tone.as_ref().is_some_and(|tone| tone.source == ToneSource::Pixels))).await;
    drop(first);

    let wide: Heard = Arc::new(Mutex::new(Vec::new()));
    let _second = rig.tones.subscribe(&source, 1.0, recording(&wide)).unwrap();
    wait_for(|| !wide.lock().unwrap().is_empty()).await;

    assert_eq!((rig.tones.analyses(), preparation.calls.load(Ordering::SeqCst)), (1, 1));
    let wide_tone = wide.lock().unwrap()[0].clone().unwrap();
    assert_eq!(wide_tone.source, ToneSource::Pixels);
    assert_eq!(wide_tone, rig.tones.tone(&source, 1.0).await.unwrap().unwrap());
}

#[tokio::test]
async fn inv_tone_19_a_cancelled_one_shot_read_leaves_no_standing_preparation_demand() {
    let rig = gated_rig(1).await;
    let source = item("https://cdn.test/oneshot.jpg", None);
    let preparation = Counted::new();
    rig.tones.set_preparation(preparation.clone());
    let tones = rig.tones.clone();
    let read = { let source = source.clone(); tokio::spawn(async move { tones.tone(&source, 0.15).await }) };
    wait_for(|| rig.store.started.load(Ordering::SeqCst) == 1).await;

    read.abort();
    wait_for(|| rig.store.dropped.load(Ordering::SeqCst) == 1).await;
    rig.store.gate.add_permits(1);
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(rig.tones.demand_count(), 0);
    assert_eq!(preparation.calls.load(Ordering::SeqCst), 0);
}
