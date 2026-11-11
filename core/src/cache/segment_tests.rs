use super::naming::{bin_path, file_name};
use super::test_support::*;
use super::verify::SEGMENT;
use super::*;
use crate::limits::EvictionStrategy;
use std::sync::atomic::Ordering;

const KEY: &str = "clip";
const ROOMY: u64 = 64 * 1024 * 1024;

fn blob() -> Vec<u8> {
    (0..SEGMENT * 4 + 100).map(|index| (index % 251) as u8).collect()
}

async fn stored_then_reopened() -> (tempfile::TempDir, Cache) {
    let f = fixture(limits(ROOMY, 0.9, EvictionStrategy::Lru), &[]).await;
    f.cache.put(KEY, Kind::Stream, &blob()).await.unwrap();
    f.cache.flush().await.unwrap();
    let cache = Cache::open(f.dir.path(), limits(ROOMY, 0.9, EvictionStrategy::Lru));
    cache.ready().await;
    (f.dir, cache)
}

fn flip(dir: &std::path::Path, offset: usize) {
    let path = bin_path(dir, &file_name(KEY));
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[offset] ^= 0xff;
    std::fs::write(path, bytes).unwrap();
}

fn hashed(cache: &Cache) -> u64 {
    cache.inner.hashed_bytes.load(Ordering::Relaxed)
}

#[tokio::test]
async fn inv_cache_31_the_first_bytes_wait_for_their_own_segment_only() {
    let (_dir, cache) = stored_then_reopened().await;

    let reader = cache.open_reader(KEY).await.unwrap();
    let head = reader.get_range(0, 10).await;

    assert_eq!(head.as_deref(), Some(&blob()[..10]));
    assert_eq!(hashed(&cache), SEGMENT as u64);
}

#[tokio::test]
async fn inv_cache_31_a_range_reader_is_checked_and_never_receives_a_damaged_segment() {
    let (dir, cache) = stored_then_reopened().await;
    flip(dir.path(), SEGMENT * 2 + 5);

    let reader = cache.open_reader(KEY).await.expect("a damaged middle does not fail the open");
    assert_eq!(reader.get_range(SEGMENT as u64 * 3, 10).await.as_deref(), Some(&blob()[SEGMENT * 3..SEGMENT * 3 + 10]));
    assert_eq!(reader.get_range(SEGMENT as u64 * 2 - 5, 20).await, None, "bytes of a damaged segment reached the reader");
    assert!(!cache.contains(KEY).await);
    wait_until(|| !bin_path(dir.path(), &file_name(KEY)).exists()).await;
}

#[tokio::test]
async fn inv_cache_31_every_byte_is_hashed_once_per_process_across_readers() {
    let (_dir, cache) = stored_then_reopened().await;
    let data = blob();

    let first = cache.open_reader(KEY).await.unwrap();
    let second = cache.open_reader(KEY).await.unwrap();
    let (a, b) = tokio::join!(first.get_range(0, 100), second.get_range(50, 100));
    assert_eq!((a.as_deref(), b.as_deref()), (Some(&data[..100]), Some(&data[50..150])));
    assert_eq!(hashed(&cache), SEGMENT as u64);

    assert_eq!(first.get_range(0, data.len() as u64).await, Some(data.clone()));
    assert_eq!(second.get_range(0, data.len() as u64).await, Some(data.clone()));
    assert_eq!(hashed(&cache), data.len() as u64);
}

#[tokio::test]
async fn inv_cache_32_the_index_keeps_one_sum_per_segment_and_drops_a_record_with_another_count() {
    let f = fixture(limits(ROOMY, 0.9, EvictionStrategy::Lru), &[]).await;
    f.cache.put(KEY, Kind::Stream, &blob()).await.unwrap();
    let entry = f.cache.inner.state().slot(&file_name(KEY)).unwrap().entry.clone();

    let mut line = String::new();
    super::persist::encode_line(KEY, &entry, &mut line);
    let mut fields: Vec<&str> = line.trim_end_matches('\n').split('\t').collect();
    let sums = fields[8].len();
    eprintln!("index_sum_bytes entry_bytes={} segments=5 sums_field_bytes={sums}", blob().len());

    assert_eq!(sums, 16 * 5);
    assert!(super::persist::decode_line(line.trim_end_matches('\n')).is_some());
    let short = &fields[8][16..];
    fields[8] = short;
    assert!(super::persist::decode_line(&fields.join("\t")).is_none());
}
