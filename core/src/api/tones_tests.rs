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

    rig.tones.submit(hashed.clone(), 4, 4, pixels(255));
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
        rig.tones.submit(local(name), 4, 4, pixels(120));
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
        rig.tones.submit(first.clone(), 4, 4, pixels(90));
    }
    wait_for_source(&rig.tones, &first, ToneSource::Pixels).await;
    for _ in 0..8 {
        rig.tones.submit(first.clone(), 4, 4, pixels(90));
    }
    rig.tones.submit(second.clone(), 4, 4, pixels(90));
    wait_for_source(&rig.tones, &second, ToneSource::Pixels).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(rig.tones.analyses(), 2);
    rig.cache.flush().await.unwrap();
    let again = Tones::new(Handle::current(), Arc::new(CacheTones::new(rig.cache.clone(), Arc::new(|url, _| url.to_string()))), 8);
    again.submit(first.clone(), 4, 4, pixels(90));
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
        rig.tones.subscribe(&watched.url, Arc::new(move |source: ToneSource| {
            heard.lock().unwrap().push(source);
            count.fetch_add(1, Ordering::SeqCst);
        }))
    };

    rig.tones.submit(other.clone(), 4, 4, pixels(10));
    rig.tones.submit(watched.clone(), 4, 4, pixels(10));
    wait_for(|| count.load(Ordering::SeqCst) == 1).await;
    rig.cache.remove(&watched.url).await;
    rig.cache.put(&watched.url, CacheKind::Image, b"replaced bytes").await.unwrap();
    rig.tones.submit(watched.clone(), 4, 4, pixels(240));
    wait_for(|| count.load(Ordering::SeqCst) == 2).await;

    assert_eq!(*heard.lock().unwrap(), vec![ToneSource::Pixels, ToneSource::Pixels]);
    drop(listener);
    rig.cache.remove(&watched.url).await;
    rig.cache.put(&watched.url, CacheKind::Image, b"third bytes").await.unwrap();
    rig.tones.submit(watched.clone(), 4, 4, pixels(60));
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

    rig.tones.submit(video.clone(), 4, 4, pixels(30));
    wait_for_source(&rig.tones, &video, ToneSource::Pixels).await;

    assert!(rig.cache.tone(&block).is_some());
    assert_eq!(rig.cache.tone(&video.url), None);
    rig.cache.purge_prefix(&format!("{}#", video.url)).await;
    assert_eq!(rig.tones.tone(&video, 0.15).await, Ok(None));
}
