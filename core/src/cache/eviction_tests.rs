use super::test_support::*;
use super::*;
use crate::limits::EvictionStrategy;

#[tokio::test]
async fn inv_cache_21_eviction_avoids_full_scans() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    f.cache.inner.scans.store(0, std::sync::atomic::Ordering::Relaxed);
    super::evict::evict_to(&f.cache.inner, 60).await;
    assert_eq!(f.cache.inner.scans.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[tokio::test]
async fn inv_cache_15_a_larger_copy_that_does_not_fit_beside_the_old_one_is_refused_and_the_old_value_stays() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
    f.put("a", Kind::Image, 60).await;

    let result = f.cache.put("a", Kind::Image, &vec![b'y'; 70]).await;

    assert_eq!(result, Err(CacheError::NotEnoughSpace));
    assert_eq!(f.cache.get("a").await, Some(payload(60)));
}

#[tokio::test]
async fn inv_cache_30_selected_version_survives_replacement() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    let name = super::naming::file_name("a");
    let guard = f.cache.inner.stripe(&name).lock().await;
    let cache = f.cache.clone();
    let task = tokio::spawn(async move { super::evict::evict_to(&cache.inner, 60).await });
    wait_until(|| f.cache.inner.eviction_backlog.load(std::sync::atomic::Ordering::Relaxed) > 0).await;
    {
        let mut state = f.cache.inner.state();
        let mut entry = state.slot(&name).unwrap().entry.clone();
        entry.last_access += 100;
        state.insert(name.clone(), "a".into(), entry, true);
    }
    drop(guard);
    task.await.unwrap();
    assert_eq!(f.cache.get("a").await, Some(payload(30)));
}

#[tokio::test]
async fn inv_cache_20_failed_unlink_retains_accounting() {
    for eviction in [false, true] {
        let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
        f.put("a", Kind::Image, 30).await;
        let path = super::naming::bin_path(f.dir.path(), &super::naming::file_name("a"));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        if eviction {
            super::evict::evict_to(&f.cache.inner, 0).await;
        } else {
            assert!(!f.cache.remove("a").await);
        }
        assert_eq!(f.cache.stats().await.total_bytes, 30);
        assert_eq!(f.cache.stats().await.evictions, 0);
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, payload(30)).unwrap();
        assert!(f.cache.remove("a").await);
        assert_eq!(f.cache.stats().await.total_bytes, 0);
    }
}

async fn three_of_thirty(strategy: EvictionStrategy) -> Fixture {
    let f = fixture(limits(100, 0.8, strategy), &[]).await;
    for key in ["a", "b", "c"] {
        f.put(key, Kind::Image, 30).await;
    }
    f
}

async fn present(f: &Fixture) -> Vec<&'static str> {
    let mut keys = Vec::new();
    for key in ["a", "b", "c", "d"] {
        if f.cache.contains(key).await {
            keys.push(key);
        }
    }
    keys
}

#[tokio::test]
async fn inv_strategy_13_a_kept_entry_and_its_blocks_are_evicted_after_every_other_entry() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
    f.put("a", Kind::Image, 30).await;
    f.put("a#1", Kind::Image, 30).await;
    f.put("b", Kind::Image, 30).await;
    f.cache.keep("a");
    f.put("c", Kind::Image, 30).await;
    assert_eq!(present(&f).await, vec!["a", "c"]);
    assert!(f.cache.contains("a#1").await);

    f.put("d", Kind::Image, 30).await;
    f.put("e", Kind::Image, 30).await;
    f.put("g", Kind::Image, 30).await;
    f.put("h", Kind::Image, 30).await;
    assert!(f.cache.contains("a").await && f.cache.contains("a#1").await);
}

#[tokio::test]
async fn inv_strategy_13_a_kept_entry_is_evicted_when_nothing_else_is_left_to_fit_the_budget() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
    f.put("a", Kind::Image, 60).await;
    f.cache.keep("a");

    f.put("b", Kind::Image, 60).await;

    assert!(!f.cache.contains("a").await);
    assert!(f.cache.contains("b").await);
}

#[tokio::test]
async fn inv_cache_08_lru_evicts_the_least_recently_read() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    f.touch("a").await;
    f.put("d", Kind::Image, 30).await;
    assert_eq!(present(&f).await, vec!["a", "c", "d"]);
}

#[tokio::test]
async fn inv_cache_08_fifo_evicts_the_oldest_created_even_if_read() {
    let f = three_of_thirty(EvictionStrategy::Fifo).await;
    f.touch("a").await;
    f.put("d", Kind::Image, 30).await;
    assert_eq!(present(&f).await, vec!["b", "c", "d"]);
}

#[tokio::test]
async fn inv_cache_08_lfu_evicts_the_least_read() {
    let f = three_of_thirty(EvictionStrategy::Lfu).await;
    f.touch("a").await;
    f.touch("a").await;
    f.touch("b").await;
    f.put("d", Kind::Image, 30).await;
    assert_eq!(present(&f).await, vec!["a", "b", "d"]);
}

#[tokio::test]
async fn inv_cache_08_largest_first_evicts_the_biggest() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::LargestFirst), &[]).await;
    f.put("a", Kind::Image, 20).await;
    f.put("b", Kind::Image, 50).await;
    f.put("c", Kind::Image, 20).await;
    f.put("d", Kind::Image, 20).await;
    assert_eq!(present(&f).await, vec!["a", "c", "d"]);
}

#[tokio::test]
async fn inv_cache_08_expire_after_evicts_the_expired_entry() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::ExpireAfter(10)), &[]).await;
    f.put("a", Kind::Image, 30).await;
    f.clock.advance_ms(6_000);
    f.put("b", Kind::Image, 30).await;
    f.clock.advance_ms(6_000);
    f.put("c", Kind::Image, 30).await;
    f.put("d", Kind::Image, 30).await;
    assert_eq!(present(&f).await, vec!["b", "c", "d"]);
}

#[tokio::test]
async fn inv_cache_08_kind_override_replaces_the_default_strategy() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[(Kind::Image, EvictionStrategy::Fifo)]).await;
    for key in ["a", "b", "c"] {
        f.put(key, Kind::Image, 30).await;
    }
    f.touch("a").await;
    f.put("d", Kind::Image, 30).await;
    assert_eq!(present(&f).await, vec!["b", "c", "d"]);
}

#[tokio::test]
async fn inv_cache_08_eviction_is_counted_in_stats() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    f.put("d", Kind::Image, 30).await;
    let stats = f.cache.stats().await;
    assert_eq!(stats.evictions, 1);
    assert_eq!(stats.total_bytes, 90);
    assert_eq!(files_with_suffix(f.dir.path(), ".bin"), 3);
}

#[tokio::test]
async fn inv_cache_04_open_reader_protects_the_entry() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
    f.put("a", Kind::Image, 60).await;
    let reader = f.cache.open_reader("a").await.unwrap();
    let result = f.cache.put("b", Kind::Image, &payload(60)).await;
    assert_eq!(result, Err(CacheError::NotEnoughSpace));
    assert!(f.cache.contains("a").await);
    drop(reader);
    f.cache.put("b", Kind::Image, &payload(60)).await.unwrap();
    assert!(!f.cache.contains("a").await);
}

#[tokio::test]
async fn inv_cache_04_open_writer_protects_the_entry() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
    f.put("a", Kind::Image, 60).await;
    let writer = f.cache.begin_write("a", Kind::Image).await.unwrap();
    let result = f.cache.put("b", Kind::Image, &payload(60)).await;
    assert_eq!(result, Err(CacheError::NotEnoughSpace));
    assert!(f.cache.contains("a").await);
    drop(writer);
}

#[tokio::test]
async fn inv_cache_01_entry_larger_than_budget_is_refused_and_leaves_no_files() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
    let result = f.cache.put("a", Kind::Image, &payload(150)).await;
    assert_eq!(result, Err(CacheError::NotEnoughSpace));
    wait_until(|| files_with_suffix(f.dir.path(), ".part") == 0).await;
    assert_eq!(f.cache.stats().await.file_count, 0);
}

#[tokio::test]
async fn inv_cache_02_failed_put_does_not_leak_reserved_space() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
    f.put("a", Kind::Image, 60).await;
    let reader = f.cache.open_reader("a").await.unwrap();
    assert!(f.cache.put("b", Kind::Image, &payload(60)).await.is_err());
    drop(reader);
    f.clock.advance_ms(10);
    f.cache.put("c", Kind::Image, &payload(40)).await.unwrap();
    assert!(f.cache.contains("c").await);
}

#[tokio::test]
async fn inv_cache_10_set_limits_below_used_evicts_immediately() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    f.cache.set_limits(limits(50, 0.8, EvictionStrategy::Lru)).await.unwrap();
    let stats = f.cache.stats().await;
    assert_eq!(stats.file_count, 1);
    assert_eq!(stats.evictions, 2);
    assert_eq!(files_with_suffix(f.dir.path(), ".bin"), 1);
    assert!(f.cache.contains("c").await);
}

#[tokio::test]
async fn inv_cache_10_set_limits_rejects_invalid_limits() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    let invalid = crate::limits::Limits { budget: 10, min_budget: 20, ..crate::limits::Limits::default() };
    assert!(f.cache.set_limits(invalid).await.is_err());
    assert_eq!(f.cache.stats().await.file_count, 3);
}

#[tokio::test]
async fn inv_cache_08_reaching_the_budget_evicts_in_background_down_to_shrink_target() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    f.put("d", Kind::Image, 10).await;
    wait_for_file_count(&f.cache, 3).await;
    assert_eq!(f.cache.stats().await.total_bytes, 70);
    assert!(!f.cache.contains("a").await);
}

#[tokio::test]
async fn inv_cache_08_evict_now_runs_when_used_is_at_budget() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    f.cache.put("d", Kind::Image, &payload(10)).await.unwrap();
    f.cache.evict_now().await;
    assert_eq!(f.cache.stats().await.total_bytes, 70);
}

#[tokio::test]
async fn inv_cache_09_eviction_waits_for_the_key_lock_before_deleting_the_file() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    let name = crate::identity::key_file_name("a");
    let guard = f.cache.inner.stripe(&name).lock().await;
    let cache = f.cache.clone();
    let evicting = tokio::spawn(async move { cache.set_limits(limits(70, 0.5, EvictionStrategy::Lru)).await });
    wait_until(|| f.cache.inner.eviction_backlog.load(std::sync::atomic::Ordering::Relaxed) > 0).await;
    assert!(f.dir.path().join(format!("{name}.bin")).exists());
    assert!(!evicting.is_finished());
    drop(guard);
    evicting.await.unwrap().unwrap();
    assert!(!f.cache.contains("a").await);
    assert_eq!(files_with_suffix(f.dir.path(), ".bin"), 1);
}

#[tokio::test]
async fn inv_cache_09_entry_rewritten_during_eviction_keeps_its_file() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    let name = crate::identity::key_file_name("a");
    let guard = f.cache.inner.stripe(&name).lock().await;
    let cache = f.cache.clone();
    let evicting = tokio::spawn(async move { cache.set_limits(limits(70, 0.5, EvictionStrategy::Lru)).await });
    wait_until(|| f.cache.inner.eviction_backlog.load(std::sync::atomic::Ordering::Relaxed) > 0).await;
    drop(guard);
    f.cache.put("a", Kind::Image, &payload(30)).await.unwrap();
    evicting.await.unwrap().unwrap();
    for key in ["a", "b", "c"] {
        assert_eq!(f.cache.contains(key).await, f.cache.get(key).await.is_some(), "{key}");
    }
}

fn mixed_priority_options(clock: std::sync::Arc<super::clock::ManualClock>) -> Options {
    Options {
        kind_priority: [Kind::Audio, Kind::Image, Kind::Stream],
        ..options(clock, &[(Kind::Audio, EvictionStrategy::LargestFirst)])
    }
}

#[tokio::test]
async fn inv_cache_08_mixed_strategies_evict_by_kind_priority_then_kind_strategy() {
    for round in 0..20 {
        let dir = tempfile::tempdir().unwrap();
        let clock = std::sync::Arc::new(super::clock::ManualClock::new(1_000));
        let cache = Cache::open_with(dir.path(), limits(100, 0.8, EvictionStrategy::Lru), mixed_priority_options(clock.clone()));
        cache.ready().await;
        for (key, kind, len) in [("i1", Kind::Image, 20), ("i2", Kind::Image, 20), ("a1", Kind::Audio, 10), ("a2", Kind::Audio, 30)] {
            let key = format!("{key}-{round}");
            let key = key.as_str();
            clock.advance_ms(10);
            cache.put(key, kind, &payload(len)).await.unwrap();
        }
        clock.advance_ms(10);
        cache.get(&format!("i1-{round}")).await.unwrap();
        cache.put(&format!("i3-{round}"), Kind::Image, &payload(30)).await.unwrap();
        assert!(!cache.contains(&format!("a2-{round}")).await);
        for key in ["i1", "i2", "a1", "i3"] {
            assert!(cache.contains(&format!("{key}-{round}")).await, "{key}");
        }
    }
}

#[tokio::test]
async fn inv_cache_08_kinds_sharing_a_strategy_are_ranked_together() {
    for _ in 0..20 {
        let dir = tempfile::tempdir().unwrap();
        let clock = std::sync::Arc::new(super::clock::ManualClock::new(1_000));
        let options = Options { kind_priority: [Kind::Audio, Kind::Image, Kind::Stream], ..options(clock.clone(), &[]) };
        let cache = Cache::open_with(dir.path(), limits(100, 0.8, EvictionStrategy::Lru), options);
        cache.ready().await;
        for (key, kind) in [("i1", Kind::Image), ("a1", Kind::Audio), ("s1", Kind::Stream)] {
            clock.advance_ms(10);
            cache.put(key, kind, &payload(30)).await.unwrap();
        }
        cache.put("i2", Kind::Image, &payload(30)).await.unwrap();
        assert!(!cache.contains("i1").await);
        for key in ["a1", "s1", "i2"] {
            assert!(cache.contains(key).await, "{key}");
        }
    }
}

fn part_bytes(dir: &std::path::Path) -> u64 {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|file| file.file_name().to_string_lossy().ends_with(".part"))
        .map(|file| file.metadata().unwrap().len())
        .sum()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inv_cache_01_two_writers_at_the_budget_edge_cannot_both_append() {
    for round in 0..50 {
        let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
        let mut first = f.cache.begin_write(&format!("a-{round}"), Kind::Stream).await.unwrap();
        let mut second = f.cache.begin_write(&format!("b-{round}"), Kind::Stream).await.unwrap();
        let (one, two) = tokio::join!(
            tokio::spawn(async move { (first.write(&[0; 70]).await, first) }),
            tokio::spawn(async move { (second.write(&[0; 70]).await, second) })
        );
        let (one, _first) = one.unwrap();
        let (two, _second) = two.unwrap();
        assert_eq!(one.is_ok() as u8 + two.is_ok() as u8, 1, "round {round}");
        assert!([one, two].contains(&Err(CacheError::NotEnoughSpace)));
        assert_eq!(part_bytes(f.dir.path()), 70);
    }
}

#[tokio::test]
async fn inv_cache_02_dropped_writer_releases_its_reserved_space() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
    let mut first = f.cache.begin_write("a", Kind::Stream).await.unwrap();
    first.write(&[0; 70]).await.unwrap();
    let mut second = f.cache.begin_write("b", Kind::Stream).await.unwrap();
    assert_eq!(second.write(&[0; 70]).await, Err(CacheError::NotEnoughSpace));
    drop(first);
    second.write(&[0; 70]).await.unwrap();
}

#[tokio::test]
async fn inv_cache_02_aborted_writer_releases_space_and_removes_its_part() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
    let mut first = f.cache.begin_write("a", Kind::Stream).await.unwrap();
    first.write(&[0; 70]).await.unwrap();
    first.abort().await;
    assert_eq!(files_with_suffix(f.dir.path(), ".part"), 0);
    f.cache.put("b", Kind::Stream, &payload(100)).await.unwrap();
}

#[tokio::test]
async fn inv_cache_02_commit_counts_the_bytes_once() {
    let f = fixture(limits(100, 1.0, EvictionStrategy::Lru), &[]).await;
    let mut first = f.cache.begin_write("a", Kind::Stream).await.unwrap();
    first.write(&[0; 60]).await.unwrap();
    first.commit().await.unwrap();
    assert_eq!(f.cache.stats().await.total_bytes, 60);
    f.cache.put("b", Kind::Stream, &payload(40)).await.unwrap();
    f.cache.evict_now().await;
    assert_eq!(f.cache.stats().await.total_bytes, 100);
    assert!(f.cache.contains("a").await);
    assert!(f.cache.contains("b").await);
}

#[tokio::test]
async fn inv_cache_01_append_evicts_unprotected_entries_to_make_room() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    let mut writer = f.cache.begin_write("d", Kind::Stream).await.unwrap();
    writer.write(&[0; 30]).await.unwrap();
    assert!(!f.cache.contains("a").await);
    writer.commit().await.unwrap();
    assert!(f.cache.contains("d").await);
}
