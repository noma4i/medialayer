use super::test_support::*;
use super::*;
use crate::limits::EvictionStrategy;
use crate::proxy::{ByteStore, Meta};
use std::sync::Arc;

const KEY: &str = "https://o.test/v.mp4#0";

async fn store() -> (CacheStore, Fixture) {
    let f = fixture(limits(10 * 1024 * 1024, 0.9, EvictionStrategy::Lru), &[]).await;
    (CacheStore::new(f.cache.clone(), Arc::new(classify_kind)), f)
}

fn meta(content_type: Option<&str>) -> Meta {
    Meta { content_type: content_type.map(str::to_string), total_length: None }
}

#[test]
fn inv_cache_13_kind_follows_the_content_type_then_the_extension_then_defaults_to_stream() {
    let cases = [
        ("https://o.test/a.bin", Some("image/jpeg"), Kind::Image),
        ("https://o.test/a.bin", Some("audio/mpeg"), Kind::Audio),
        ("https://o.test/a.bin", Some("video/mp4"), Kind::Stream),
        ("https://o.test/a.m3u8", Some("application/vnd.apple.mpegurl"), Kind::Stream),
        ("https://o.test/photo.JPG#2", Some("application/octet-stream"), Kind::Image),
        ("https://o.test/voice.m4a#3", None, Kind::Audio),
        ("https://o.test/clip", None, Kind::Stream),
    ];
    for (key, content_type, expected) in cases {
        assert_eq!(classify_kind(key, &meta(content_type)), expected, "{key} {content_type:?}");
    }
}

#[tokio::test]
async fn inv_cache_12_an_open_write_is_readable_before_the_commit_and_complete_after_it() {
    let (store, f) = store().await;
    let described = Meta { content_type: Some("video/mp4".to_string()), total_length: Some(6) };
    let mut writer = store.begin_write(KEY, described.clone()).await.unwrap();

    assert!(writer.append(b"abc").await);
    assert_eq!(store.get_range(KEY, 0, 10).await, Some(b"abc".to_vec()));
    assert_eq!(store.get_range(KEY, 1, 1).await, Some(b"b".to_vec()));
    assert_eq!((store.len(KEY).await, store.contains(KEY).await, store.meta(KEY).await), (None, false, None));

    assert!(writer.append(b"def").await);
    assert_eq!(store.get_range(KEY, 2, 3).await, Some(b"cde".to_vec()));
    assert!(writer.commit().await);

    assert_eq!(store.get_range(KEY, 0, 10).await, Some(b"abcdef".to_vec()));
    assert_eq!((store.len(KEY).await, store.meta(KEY).await), (Some(6), Some(described)));
    assert_eq!(f.cache.stats().await.per_kind[&Kind::Stream].files, 1);
}

#[tokio::test]
async fn inv_cache_12_an_aborted_write_leaves_nothing_readable_and_no_part_file() {
    let (store, f) = store().await;
    let mut writer = store.begin_write(KEY, Meta::default()).await.unwrap();
    assert!(writer.append(b"zz").await);
    writer.abort().await;

    assert_eq!(store.get_range(KEY, 0, 10).await, None);
    wait_until(|| files_with_suffix(f.dir.path(), ".part") == 0).await;
    assert_eq!(f.cache.stats().await.open_writers, 0);
}

#[tokio::test]
async fn inv_cache_34_the_open_of_an_unfinished_write_runs_off_the_polling_thread() {
    let (store, f) = store().await;

    let writer = store.begin_write(KEY, Meta::default()).await.unwrap();

    let opened = f.cache.inner.write_open_threads.lock().unwrap().clone();
    assert_eq!(opened.len(), 1);
    assert_ne!(opened[0], std::thread::current().id());
    writer.abort().await;
}

#[tokio::test]
async fn inv_cache_34_cancelling_begin_write_inside_the_open_keeps_the_part_until_the_open_and_leaves_nothing_after() {
    let (store, f) = store().await;
    let (started_sender, started) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    *f.cache.inner.write_open_pause.lock().unwrap() = Some((started_sender, released));
    let call = {
        let store = store.clone();
        tokio::spawn(async move { store.begin_write(KEY, Meta::default()).await.is_some() })
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), started).await.expect("the open never started").unwrap();

    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());

    assert_eq!(f.cache.inner.writer_drops.load(std::sync::atomic::Ordering::Relaxed), 0, "the cancel dropped the writer before its open");
    assert_eq!(files_with_suffix(f.dir.path(), ".part"), 1);
    assert_eq!(store.open_writes(), 0);
    release.send(()).unwrap();
    wait_until(|| store.open_writes() == 0 && files_with_suffix(f.dir.path(), ".part") == 0).await;
    assert_eq!(f.cache.inner.writer_drops.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(f.cache.inner.write_open_threads.lock().unwrap().len(), 1, "the open of the abandoned write did not succeed");
}
