use super::naming::file_name;
use super::test_support::*;
use super::*;
use crate::limits::EvictionStrategy;

const MAX_POLLS: usize = 8;

fn reserved(cache: &Cache) -> u64 {
    cache.inner.state().reserved
}

#[tokio::test]
async fn inv_cache_03_open_reader_cancelled_at_any_await_point_releases_its_pin() {
    for polls in 1..=MAX_POLLS {
        let f = fixture(limits(100, 0.9, EvictionStrategy::Lru), &[]).await;
        f.put("a", Kind::Image, 40).await;
        poll_then_drop(f.cache.open_reader("a"), polls).await;
        f.cache.put("b", Kind::Image, &payload(70)).await.unwrap_or_else(|error| panic!("polls {polls}: {error}"));
    }
}

#[tokio::test]
async fn inv_cache_02_begin_write_cancelled_at_any_await_point_unregisters_the_writer() {
    for polls in 1..=MAX_POLLS {
        let f = fixture(limits(100, 0.9, EvictionStrategy::Lru), &[]).await;
        if poll_then_drop(f.cache.begin_write("a", Kind::Stream), polls).await {
            continue;
        }
        wait_until(|| files_with_suffix(f.dir.path(), ".part") == 0).await;
        assert_eq!(f.cache.stats().await.open_writers, 0, "polls {polls}");
        assert!(f.cache.inner.state().live_parts.is_empty(), "polls {polls}");
    }
}

#[tokio::test]
async fn inv_cache_02_write_cancelled_at_any_await_point_leaves_no_reserve_after_the_writer_drops() {
    for polls in 1..=MAX_POLLS {
        let f = fixture(limits(100, 0.9, EvictionStrategy::Lru), &[]).await;
        let mut writer = f.cache.begin_write("a", Kind::Stream).await.unwrap();
        poll_then_drop(writer.write(&[0; 80]), polls).await;
        drop(writer);
        let mut next = f.cache.begin_write("b", Kind::Stream).await.unwrap();
        next.write(&[1; 100]).await.unwrap_or_else(|error| panic!("polls {polls}: {error}"));
    }
}

#[tokio::test]
async fn inv_cache_02_commit_cancelled_at_any_await_point_leaves_a_consistent_cache() {
    for polls in 1..=MAX_POLLS {
        let f = fixture(limits(100, 0.9, EvictionStrategy::Lru), &[]).await;
        let mut writer = f.cache.begin_write("a", Kind::Stream).await.unwrap();
        writer.write(&[0; 80]).await.unwrap();
        poll_then_drop(writer.commit(), polls).await;
        wait_until(|| reserved(&f.cache) == 0 && files_with_suffix(f.dir.path(), ".part") == 0).await;
        let stats = f.cache.stats().await;
        assert_eq!(stats.open_writers, 0, "polls {polls}");
        assert_eq!(stats.file_count, files_with_suffix(f.dir.path(), ".bin") as u64, "polls {polls}");
        assert_eq!(stats.total_bytes, stats.file_count * 80, "polls {polls}");
    }
}

#[tokio::test]
async fn inv_cache_02_commit_cancelled_while_waiting_for_the_key_lock_releases_the_reserve() {
    let f = fixture(limits(100, 0.9, EvictionStrategy::Lru), &[]).await;
    let mut writer = f.cache.begin_write("a", Kind::Stream).await.unwrap();
    writer.write(&[0; 80]).await.unwrap();
    let stripe = f.cache.inner.stripe(&file_name("a")).lock().await;
    let task = tokio::spawn(writer.commit());
    wait_until(|| f.cache.inner.commit_waiting.load(std::sync::atomic::Ordering::SeqCst) > 0).await;
    task.abort();
    let _ = task.await;
    drop(stripe);
    assert_eq!(reserved(&f.cache), 0);
    assert_eq!(f.cache.stats().await.open_writers, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inv_cache_02_a_begin_write_cancelled_at_any_microsecond_leaves_no_part_file() {
    let f = fixture(limits(1 << 30, 0.9, EvictionStrategy::Lru), &[]).await;
    for round in 0..4000u64 {
        let key = format!("k{round}");
        let _ = tokio::time::timeout(std::time::Duration::from_micros(round % 120), f.cache.begin_write(&key, Kind::Stream)).await;
    }
    wait_until(|| f.cache.inner.pending_parts.load(std::sync::atomic::Ordering::SeqCst) == 0).await;
    assert_eq!(f.cache.stats().await.open_writers, 0);
    assert_eq!(files_with_suffix(f.dir.path(), ".part"), 0, "cancelled begin_write calls left part files");
}
#[tokio::test]
async fn inv_cache_27_purge_prevents_late_writer_commit() {
    for prefix in [false, true] {
        let f = super::test_support::fixture(crate::limits::Limits::default(), &[]).await;
        let mut writer = f.cache.begin_write("clip#0", super::Kind::Stream).await.unwrap();
        writer.write(b"stale").await.unwrap();
        if prefix {
            f.cache.purge_prefix("clip#").await;
        } else {
            f.cache.clear(None).await;
        }
        assert_eq!(writer.commit().await, Err(super::CacheError::Purged));
        assert!(!f.cache.contains("clip#0").await);
        f.cache.put("clip#0", super::Kind::Stream, b"fresh").await.unwrap();
        assert_eq!(f.cache.get("clip#0").await.as_deref(), Some(b"fresh".as_slice()));
    }
}
