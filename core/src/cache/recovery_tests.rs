use super::test_support::*;
use super::*;
use crate::limits::EvictionStrategy;

fn roomy() -> crate::limits::Limits {
    limits(10 * 1024 * 1024 * 1024, 0.9, EvictionStrategy::Lru)
}

#[tokio::test]
async fn inv_cache_22_startup_defers_file_metadata_until_read() {
    let f = fixture(roomy(), &[]).await;
    f.put("k", Kind::Stream, 30).await;
    f.cache.flush().await.unwrap();
    let cache = Cache::open(f.dir.path(), roomy(), u64::MAX);
    cache.ready().await;
    assert_eq!(cache.inner.load_stats.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(cache.get("k").await, Some(payload(30)));
}

#[tokio::test]
async fn inv_cache_07_index_survives_a_reopen() {
    let f = fixture(roomy(), &[]).await;
    f.put("k", Kind::Audio, 25).await;
    f.cache.flush().await.unwrap();
    let reopened = Cache::open(f.dir.path(), roomy(), u64::MAX);
    reopened.ready().await;
    assert_eq!(reopened.get("k").await, Some(payload(25)));
    let stats = reopened.stats().await;
    assert_eq!(stats.per_kind[&Kind::Audio], KindStats { bytes: 25, files: 1 });
}

#[tokio::test]
async fn inv_cache_07_part_files_are_removed_on_open() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(format!("{}.7.part", crate::identity::key_file_name("ab"))), b"partial").unwrap();
    let cache = Cache::open(dir.path(), roomy(), u64::MAX);
    cache.ready().await;
    wait_until(|| files_with_suffix(dir.path(), ".part") == 0).await;
}

#[tokio::test]
async fn inv_cache_07_orphan_files_without_an_index_entry_are_removed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(format!("{}.bin", crate::identity::key_file_name("ab"))), b"orphan").unwrap();
    let cache = Cache::open(dir.path(), roomy(), u64::MAX);
    cache.ready().await;
    assert!(!cache.contains("ab").await);
    wait_until(|| files_with_suffix(dir.path(), ".bin") == 0).await;
}

#[tokio::test]
async fn inv_cache_07_index_entries_without_a_file_are_dropped() {
    let f = fixture(roomy(), &[]).await;
    f.put("k", Kind::Image, 10).await;
    f.cache.flush().await.unwrap();
    std::fs::remove_file(f.dir.path().join(format!("{}.bin", crate::identity::key_file_name("k")))).unwrap();
    let reopened = Cache::open(f.dir.path(), roomy(), u64::MAX);
    reopened.ready().await;
    assert!(!reopened.contains("k").await);
    assert_eq!(reopened.stats().await.file_count, 0);
}

#[tokio::test]
async fn inv_cache_07_live_writer_part_survives_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), roomy(), u64::MAX);
    let mut writer = cache.begin_write("k", Kind::Stream).await.unwrap();
    writer.write(b"abc").await.unwrap();
    cache.ready().await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(files_with_suffix(dir.path(), ".part"), 1);
    writer.commit().await.unwrap();
    assert_eq!(cache.get("k").await, Some(b"abc".to_vec()));
}

#[tokio::test]
async fn inv_cache_07_entries_over_budget_after_load_are_evicted() {
    let f = fixture(roomy(), &[]).await;
    for key in ["a", "b", "c"] {
        f.put(key, Kind::Image, 30).await;
    }
    f.cache.flush().await.unwrap();
    let reopened = Cache::open(f.dir.path(), limits(60, 0.5, EvictionStrategy::Lru), u64::MAX);
    reopened.ready().await;
    wait_for_file_count(&reopened, 1).await;
}

fn bin_of(dir: &std::path::Path, key: &str) -> std::path::PathBuf {
    super::naming::bin_path(dir, &super::naming::file_name(key))
}

async fn reopened_after(damage: impl Fn(&mut Vec<u8>)) -> (tempfile::TempDir, Cache) {
    let f = fixture(roomy(), &[]).await;
    f.put("good", Kind::Stream, 1000).await;
    f.put("bad", Kind::Stream, 1000).await;
    f.cache.flush().await.unwrap();
    let path = bin_of(f.dir.path(), "bad");
    let mut bytes = std::fs::read(&path).unwrap();
    damage(&mut bytes);
    std::fs::write(&path, bytes).unwrap();
    let reopened = Cache::open(f.dir.path(), roomy(), u64::MAX);
    reopened.ready().await;
    (f.dir, reopened)
}

#[tokio::test]
async fn inv_cache_17_a_damaged_file_is_never_served_and_is_dropped_on_first_touch() {
    let damages: [(&str, Box<dyn Fn(&mut Vec<u8>)>); 4] = [
        ("flipped byte", Box::new(|bytes| bytes[500] ^= 0xff)),
        ("zeroed tail", Box::new(|bytes| bytes[600..].fill(0))),
        ("torn short", Box::new(|bytes| bytes.truncate(600))),
        ("garbage appended", Box::new(|bytes| bytes.extend_from_slice(b"junk"))),
    ];
    for (name, damage) in damages {
        let (dir, cache) = reopened_after(damage).await;
        assert!(cache.get("good").await == Some(payload(1000)), "{name}");
        assert_eq!(cache.get("bad").await, None, "{name}");
        assert!(!cache.contains("bad").await, "{name}");
        assert_eq!(cache.stats().await.file_count, 1, "{name}");
        wait_until(|| !bin_of(dir.path(), "bad").exists()).await;
    }
}

#[tokio::test]
async fn inv_cache_24_unversioned_entries_are_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let mut index = String::new();
    for (key, indexed, on_disk) in [("fine", 100usize, 100usize), ("short", 100, 40), ("other", 60, 60)] {
        std::fs::write(bin_of(dir.path(), key), vec![b'q'; on_disk]).unwrap();
        index.push_str(&format!("{}\t{indexed}\tstream\t1\t2\t3\t\t\n", super::naming::hex_encode(key)));
    }
    std::fs::write(dir.path().join("index.tsv"), index).unwrap();
    let cache = Cache::open(dir.path(), roomy(), u64::MAX);
    cache.ready().await;

    assert!(cache.get("fine").await.is_none());
    assert!(!cache.contains("short").await);
    assert_eq!(cache.stats().await.file_count, 0);
    cache.flush().await.unwrap();

    let saved = std::fs::read_to_string(dir.path().join("index.tsv")).unwrap();
    assert_eq!(saved, super::persist::HEADER);
}
