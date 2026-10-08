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
