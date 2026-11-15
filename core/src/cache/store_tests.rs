use super::test_support::*;
use super::*;
use crate::limits::EvictionStrategy;

const CHUNK: usize = 16 * 1024;
const STREAM_SIZE: usize = 5 * 1024 * 1024;

async fn roomy() -> Fixture {
    fixture(limits(10 * 1024 * 1024 * 1024, 0.9, EvictionStrategy::Lru), &[]).await
}

#[tokio::test]
async fn inv_cache_05_put_then_get_returns_the_bytes() {
    let f = roomy().await;
    f.cache.put("k", Kind::Image, b"hello world").await.unwrap();
    assert_eq!(f.cache.get("k").await, Some(b"hello world".to_vec()));
}

#[tokio::test]
async fn inv_cache_05_get_of_missing_key_is_a_miss() {
    let f = roomy().await;
    assert_eq!(f.cache.get("nope").await, None);
}

#[tokio::test]
async fn inv_cache_05_get_range_returns_the_slice() {
    let f = roomy().await;
    f.cache.put("k", Kind::Stream, b"0123456789").await.unwrap();
    assert_eq!(f.cache.get_range("k", 3, 4).await, Some(b"3456".to_vec()));
    assert_eq!(f.cache.get_range("k", 8, 100).await, Some(b"89".to_vec()));
    assert_eq!(f.cache.get_range("k", 50, 4).await, Some(Vec::new()));
}

#[tokio::test]
async fn inv_cache_05_file_name_is_the_identity_hash_of_the_key() {
    let f = roomy().await;
    f.cache.put("ab", Kind::Image, b"x").await.unwrap();
    let name = crate::identity::key_file_name("ab");
    assert_eq!(name.len(), 64);
    assert!(f.dir.path().join(format!("{name}.bin")).exists());
}

#[tokio::test]
async fn inv_cache_05_put_replaces_the_previous_value_and_size() {
    let f = roomy().await;
    f.put("k", Kind::Image, 100).await;
    f.put("k", Kind::Image, 40).await;
    let stats = f.cache.stats().await;
    assert_eq!((stats.file_count, stats.total_bytes), (1, 40));
}

#[tokio::test]
async fn inv_cache_05_contains_and_remove() {
    let f = roomy().await;
    f.put("k", Kind::Image, 10).await;
    assert!(f.cache.contains("k").await);
    assert!(f.cache.remove("k").await);
    assert!(!f.cache.contains("k").await);
    assert!(!f.cache.remove("k").await);
    assert_eq!(files_with_suffix(f.dir.path(), ".bin"), 0);
}

#[tokio::test]
async fn inv_cache_05_purge_prefix_removes_only_matching_keys() {
    let f = roomy().await;
    for key in ["video/1/a", "video/1/b", "video/2/a", "image/1"] {
        f.put(key, Kind::Stream, 10).await;
    }
    assert_eq!(f.cache.purge_prefix("video/1/").await, 2);
    assert!(!f.cache.contains("video/1/a").await);
    assert!(f.cache.contains("video/2/a").await);
    assert!(f.cache.contains("image/1").await);
    assert_eq!(files_with_suffix(f.dir.path(), ".bin"), 2);
    assert_eq!(f.cache.stats().await.total_bytes, 20);
}

#[tokio::test]
async fn inv_cache_13_clear_by_kind_keeps_other_kinds() {
    let f = roomy().await;
    f.put("i", Kind::Image, 10).await;
    f.put("s", Kind::Stream, 20).await;
    f.put("a", Kind::Audio, 30).await;
    assert_eq!(f.cache.clear(Some(Kind::Stream)).await, 1);
    assert!(f.cache.contains("i").await);
    assert!(!f.cache.contains("s").await);
    assert!(f.cache.contains("a").await);
    assert_eq!(f.cache.stats().await.total_bytes, 40);
}

#[tokio::test]
async fn inv_cache_05_clear_all_empties_the_cache() {
    let f = roomy().await;
    f.put("i", Kind::Image, 10).await;
    f.put("s", Kind::Stream, 20).await;
    assert_eq!(f.cache.clear(None).await, 2);
    let stats = f.cache.stats().await;
    assert_eq!((stats.total_bytes, stats.file_count), (0, 0));
    assert_eq!(files_with_suffix(f.dir.path(), ".bin"), 0);
}

#[tokio::test]
async fn inv_cache_13_stats_split_bytes_by_kind() {
    let f = roomy().await;
    f.put("i", Kind::Image, 10).await;
    f.put("s", Kind::Stream, 20).await;
    f.put("a", Kind::Audio, 30).await;
    let stats = f.cache.stats().await;
    assert_eq!((stats.total_bytes, stats.file_count), (60, 3));
    assert_eq!(stats.per_kind[&Kind::Image], KindStats { bytes: 10, files: 1 });
    assert_eq!(stats.per_kind[&Kind::Stream], KindStats { bytes: 20, files: 1 });
    assert_eq!(stats.per_kind[&Kind::Audio], KindStats { bytes: 30, files: 1 });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inv_cache_12_parallel_puts_of_one_key_leave_one_complete_value() {
    let f = roomy().await;
    let first = vec![1u8; 1000];
    let second = vec![2u8; 2000];
    let (a, b) = tokio::join!(f.cache.put("k", Kind::Image, &first), f.cache.put("k", Kind::Image, &second));
    a.unwrap();
    b.unwrap();
    let value = f.cache.get("k").await.unwrap();
    assert!(value == first || value == second);
    let stats = f.cache.stats().await;
    assert_eq!((stats.file_count, stats.total_bytes), (1, value.len() as u64));
    assert_eq!(files_with_suffix(f.dir.path(), ".bin"), 1);
    assert_eq!(files_with_suffix(f.dir.path(), ".part"), 0);
}

#[tokio::test]
async fn inv_cache_12_streamed_write_is_invisible_until_commit_and_read_in_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let options = Options { chunk_size: CHUNK, ..Options::default() };
    let cache = Cache::open_with(dir.path(), limits(u64::MAX / 2, 0.9, EvictionStrategy::Lru), options);
    cache.ready().await;
    let mut writer = cache.begin_write("seg", Kind::Stream).await.unwrap();
    for _ in 0..(STREAM_SIZE / CHUNK) {
        writer.write(&payload(CHUNK)).await.unwrap();
    }
    assert!(!cache.contains("seg").await);
    assert_eq!(files_with_suffix(dir.path(), ".part"), 1);
    writer.commit().await.unwrap();
    assert_eq!(files_with_suffix(dir.path(), ".part"), 0);

    let mut reader = cache.open_reader("seg").await.unwrap();
    assert_eq!(reader.len(), STREAM_SIZE as u64);
    let (mut total, mut chunks) = (0, 0);
    loop {
        let chunk = reader.read_chunk().await.unwrap();
        if chunk.is_empty() {
            break;
        }
        assert!(chunk.len() <= CHUNK);
        total += chunk.len();
        chunks += 1;
    }
    assert_eq!(total, STREAM_SIZE);
    assert_eq!(chunks, STREAM_SIZE / CHUNK);
}

#[tokio::test]
async fn inv_cache_12_dropped_writer_removes_its_part_and_keeps_the_old_value() {
    let f = roomy().await;
    f.cache.put("k", Kind::Image, b"old").await.unwrap();
    let mut writer = f.cache.begin_write("k", Kind::Image).await.unwrap();
    writer.write(b"new").await.unwrap();
    drop(writer);
    wait_until(|| files_with_suffix(f.dir.path(), ".part") == 0).await;
    assert_eq!(f.cache.get("k").await, Some(b"old".to_vec()));
}

#[tokio::test]
async fn inv_cache_14_url_key_of_5000_chars_round_trips_through_a_reopen() {
    let f = roomy().await;
    let key = format!("https://cdn.example.com/{}", "segment/".repeat(625));
    assert!(key.len() >= 5000);
    f.cache.put(&key, Kind::Stream, b"payload").await.unwrap();
    assert_eq!(f.cache.get(&key).await, Some(b"payload".to_vec()));
    assert_eq!(f.cache.purge_prefix("https://cdn.example.com/segment/").await, 1);
    f.cache.put(&key, Kind::Stream, b"payload").await.unwrap();
    f.cache.flush().await.unwrap();
    let reopened = Cache::open(f.dir.path(), limits(u64::MAX / 2, 0.9, EvictionStrategy::Lru));
    reopened.ready().await;
    assert_eq!(reopened.get(&key).await, Some(b"payload".to_vec()));
    assert_eq!(files_with_suffix(f.dir.path(), ".bin"), 1);
}

#[tokio::test]
async fn inv_cache_05_read_frequency_survives_reopening_for_lfu() {
    let limits = limits(100, 0.8, EvictionStrategy::Lfu);
    let f = fixture(limits, &[]).await;
    for key in ["a", "b", "c"] {
        f.put(key, Kind::Image, 30).await;
    }
    f.touch("a").await;
    f.touch("a").await;
    f.touch("b").await;
    f.cache.flush().await.unwrap();
    let reopened = Cache::open(f.dir.path(), limits);
    reopened.ready().await;
    reopened.put("d", Kind::Image, &payload(30)).await.unwrap();
    assert!(reopened.contains("a").await);
    assert!(reopened.contains("b").await);
    assert!(!reopened.contains("c").await);
    assert!(reopened.contains("d").await);
}
