use super::test_support::*;
use super::*;
use crate::limits::EvictionStrategy;

fn roomy() -> crate::limits::Limits {
    limits(10 * 1024 * 1024 * 1024, 0.9, EvictionStrategy::Lru)
}

#[tokio::test]
async fn inv_cache_07_index_survives_a_reopen() {
    let f = fixture(roomy(), &[]).await;
    f.put("k", Kind::Audio, 25).await;
    f.cache.flush().await;
    let reopened = Cache::open(f.dir.path(), roomy());
    reopened.ready().await;
    assert_eq!(reopened.get("k").await, Some(payload(25)));
    let stats = reopened.stats().await;
    assert_eq!(stats.per_kind[&Kind::Audio], KindStats { bytes: 25, files: 1 });
}

#[tokio::test]
async fn inv_cache_07_part_files_are_removed_on_open() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(format!("{}.7.part", crate::identity::key_file_name("ab"))), b"partial").unwrap();
    let cache = Cache::open(dir.path(), roomy());
    cache.ready().await;
    wait_until(|| files_with_suffix(dir.path(), ".part") == 0).await;
}

#[tokio::test]
async fn inv_cache_07_orphan_files_without_an_index_entry_are_removed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(format!("{}.bin", crate::identity::key_file_name("ab"))), b"orphan").unwrap();
    let cache = Cache::open(dir.path(), roomy());
    cache.ready().await;
    assert!(!cache.contains("ab").await);
    wait_until(|| files_with_suffix(dir.path(), ".bin") == 0).await;
}

#[tokio::test]
async fn inv_cache_07_index_entries_without_a_file_are_dropped() {
    let f = fixture(roomy(), &[]).await;
    f.put("k", Kind::Image, 10).await;
    f.cache.flush().await;
    std::fs::remove_file(f.dir.path().join(format!("{}.bin", crate::identity::key_file_name("k")))).unwrap();
    let reopened = Cache::open(f.dir.path(), roomy());
    reopened.ready().await;
    assert!(!reopened.contains("k").await);
    assert_eq!(reopened.stats().await.file_count, 0);
}

#[tokio::test]
async fn inv_cache_07_live_writer_part_survives_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), roomy());
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
    f.cache.flush().await;
    let reopened = Cache::open(f.dir.path(), limits(60, 0.5, EvictionStrategy::Lru));
    reopened.ready().await;
    wait_for_file_count(&reopened, 1).await;
}
