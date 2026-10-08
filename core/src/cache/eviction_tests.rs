use super::test_support::*;
use super::*;
use crate::limits::EvictionStrategy;

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
async fn lru_evicts_the_least_recently_read() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    f.touch("a").await;
    f.put("d", Kind::Image, 30).await;
    assert_eq!(present(&f).await, vec!["a", "c", "d"]);
}

#[tokio::test]
async fn fifo_evicts_the_oldest_created_even_if_read() {
    let f = three_of_thirty(EvictionStrategy::Fifo).await;
    f.touch("a").await;
    f.put("d", Kind::Image, 30).await;
    assert_eq!(present(&f).await, vec!["b", "c", "d"]);
}

#[tokio::test]
async fn lfu_evicts_the_least_read() {
    let f = three_of_thirty(EvictionStrategy::Lfu).await;
    f.touch("a").await;
    f.touch("a").await;
    f.touch("b").await;
    f.put("d", Kind::Image, 30).await;
    assert_eq!(present(&f).await, vec!["a", "b", "d"]);
}

#[tokio::test]
async fn largest_first_evicts_the_biggest() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::LargestFirst), &[]).await;
    f.put("a", Kind::Image, 20).await;
    f.put("b", Kind::Image, 50).await;
    f.put("c", Kind::Image, 20).await;
    f.put("d", Kind::Image, 20).await;
    assert_eq!(present(&f).await, vec!["a", "c", "d"]);
}

#[tokio::test]
async fn expire_after_evicts_the_expired_entry() {
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
async fn kind_override_replaces_the_default_strategy() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[(Kind::Image, EvictionStrategy::Fifo)]).await;
    for key in ["a", "b", "c"] {
        f.put(key, Kind::Image, 30).await;
    }
    f.touch("a").await;
    f.put("d", Kind::Image, 30).await;
    assert_eq!(present(&f).await, vec!["b", "c", "d"]);
}

#[tokio::test]
async fn eviction_is_counted_in_stats() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    f.put("d", Kind::Image, 30).await;
    let stats = f.cache.stats().await;
    assert_eq!(stats.evictions, 1);
    assert_eq!(stats.total_bytes, 90);
    assert_eq!(files_with_suffix(f.dir.path(), ".bin"), 3);
}

#[tokio::test]
async fn open_reader_protects_the_entry() {
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
async fn open_writer_protects_the_entry() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
    f.put("a", Kind::Image, 60).await;
    let writer = f.cache.begin_write("a", Kind::Image).await.unwrap();
    let result = f.cache.put("b", Kind::Image, &payload(60)).await;
    assert_eq!(result, Err(CacheError::NotEnoughSpace));
    assert!(f.cache.contains("a").await);
    drop(writer);
}

#[tokio::test]
async fn entry_larger_than_budget_is_refused_and_leaves_no_files() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
    let result = f.cache.put("a", Kind::Image, &payload(150)).await;
    assert_eq!(result, Err(CacheError::NotEnoughSpace));
    wait_until(|| files_with_suffix(f.dir.path(), ".part") == 0).await;
    assert_eq!(f.cache.stats().await.file_count, 0);
}

#[tokio::test]
async fn failed_put_does_not_leak_reserved_space() {
    let f = fixture(limits(100, 0.8, EvictionStrategy::Lru), &[]).await;
    f.put("a", Kind::Image, 60).await;
    let reader = f.cache.open_reader("a").await.unwrap();
    assert!(f.cache.put("b", Kind::Image, &payload(60)).await.is_err());
    drop(reader);
    f.cache.put("c", Kind::Image, &payload(40)).await.unwrap();
    assert!(f.cache.contains("c").await);
}

#[tokio::test]
async fn set_limits_below_used_evicts_immediately() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    f.cache.set_limits(limits(50, 0.8, EvictionStrategy::Lru)).await.unwrap();
    let stats = f.cache.stats().await;
    assert_eq!(stats.file_count, 1);
    assert_eq!(stats.evictions, 2);
    assert_eq!(files_with_suffix(f.dir.path(), ".bin"), 1);
    assert!(f.cache.contains("c").await);
}

#[tokio::test]
async fn set_limits_rejects_invalid_limits() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    let invalid = crate::limits::Limits { budget: 10, min_budget: 20, ..crate::limits::Limits::default() };
    assert!(f.cache.set_limits(invalid).await.is_err());
    assert_eq!(f.cache.stats().await.file_count, 3);
}

#[tokio::test]
async fn reaching_the_budget_evicts_in_background_down_to_shrink_target() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    f.put("d", Kind::Image, 10).await;
    wait_for_file_count(&f.cache, 3).await;
    assert_eq!(f.cache.stats().await.total_bytes, 70);
    assert!(!f.cache.contains("a").await);
}

#[tokio::test]
async fn evict_now_runs_when_used_is_at_budget() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    f.cache.put("d", Kind::Image, &payload(10)).await.unwrap();
    f.cache.evict_now().await;
    assert_eq!(f.cache.stats().await.total_bytes, 70);
}

#[tokio::test]
async fn eviction_waits_for_the_key_lock_before_deleting_the_file() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    let name = crate::identity::key_file_name("a");
    let guard = f.cache.inner.stripe(&name).lock().await;
    let cache = f.cache.clone();
    let evicting = tokio::spawn(async move { cache.set_limits(limits(70, 0.5, EvictionStrategy::Lru)).await });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(f.dir.path().join(format!("{name}.bin")).exists());
    assert!(!evicting.is_finished());
    drop(guard);
    evicting.await.unwrap().unwrap();
    assert!(!f.cache.contains("a").await);
    assert_eq!(files_with_suffix(f.dir.path(), ".bin"), 1);
}

#[tokio::test]
async fn entry_rewritten_during_eviction_keeps_its_file() {
    let f = three_of_thirty(EvictionStrategy::Lru).await;
    let name = crate::identity::key_file_name("a");
    let guard = f.cache.inner.stripe(&name).lock().await;
    let cache = f.cache.clone();
    let evicting = tokio::spawn(async move { cache.set_limits(limits(70, 0.5, EvictionStrategy::Lru)).await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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
async fn mixed_strategies_evict_by_kind_priority_then_kind_strategy() {
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
async fn kinds_sharing_a_strategy_are_ranked_together() {
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
