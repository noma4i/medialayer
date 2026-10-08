use super::test_support::*;
use super::*;
use crate::limits::EvictionStrategy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MIB: usize = 1024 * 1024;
const TICK: Duration = Duration::from_millis(5);
const MAX_STALL: Duration = Duration::from_millis(100);
const OPEN_LIMIT: Duration = Duration::from_millis(50);
const OPEN_FILES: usize = 20_000;
const EVICTED_ENTRIES: usize = 10_000;
const BIG_INDEX_ENTRIES: usize = 100_000;
const MAX_HOLD: Duration = Duration::from_millis(30);

fn spawn_ticker(stop: Arc<AtomicBool>) -> tokio::task::JoinHandle<(Duration, u32)> {
    tokio::spawn(async move {
        let (mut worst, mut ticks) = (Duration::ZERO, 0);
        while !stop.load(Ordering::SeqCst) {
            let before = Instant::now();
            tokio::time::sleep(TICK).await;
            worst = worst.max(before.elapsed().saturating_sub(TICK));
            ticks += 1;
        }
        (worst, ticks)
    })
}

fn write_prepared_dir(dir: &std::path::Path, files: usize, hot_key: &str) {
    let mut index = String::new();
    for number in 0..files {
        let key = format!("entry-{number}");
        std::fs::write(dir.join(format!("{}.bin", crate::identity::key_file_name(&key))), b"0123456789").unwrap();
        let entry = super::entry::Entry { size: 10, kind: Kind::Image, created: number as u64, last_access: number as u64, hits: 0, meta: Default::default() };
        index.push_str(&super::persist::encode_line(&key, &entry));
    }
    std::fs::write(dir.join(format!("{}.bin", crate::identity::key_file_name(hot_key))), b"0123456789").unwrap();
    let hot = super::entry::Entry { size: 10, kind: Kind::Image, created: 0, last_access: u64::MAX / 2, hits: 0, meta: Default::default() };
    index.push_str(&super::persist::encode_line(hot_key, &hot));
    std::fs::write(dir.join("index.tsv"), index).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn put_and_get_of_50_mib_do_not_stall_other_tasks() {
    let f = fixture(limits(u64::MAX / 2, 0.9, EvictionStrategy::Lru), &[]).await;
    let stop = Arc::new(AtomicBool::new(false));
    let ticker = spawn_ticker(stop.clone());
    let data = vec![7u8; 50 * MIB];
    f.cache.put("big", Kind::Stream, &data).await.unwrap();
    let read = f.cache.get("big").await.unwrap();
    stop.store(true, Ordering::SeqCst);
    let (worst, ticks) = ticker.await.unwrap();
    assert_eq!(read.len(), data.len());
    assert!(ticks > 3, "ticker barely ran: {ticks}");
    assert!(worst < MAX_STALL, "caller thread stalled for {worst:?}");
}

#[tokio::test(flavor = "current_thread")]
async fn open_on_20000_files_returns_fast_and_reads_miss_until_ready() {
    let dir = tempfile::tempdir().unwrap();
    write_prepared_dir(dir.path(), OPEN_FILES, "hot");
    let started = Instant::now();
    let cache = Cache::open(dir.path(), limits(u64::MAX / 2, 0.9, EvictionStrategy::Lru));
    let elapsed = started.elapsed();
    assert!(elapsed < OPEN_LIMIT, "open took {elapsed:?}");
    assert!(!cache.is_ready());
    assert_eq!(cache.get("hot").await, None);
    assert!(!cache.contains("hot").await);
    cache.ready().await;
    assert_eq!(cache.get("hot").await, Some(b"0123456789".to_vec()));
    assert_eq!(cache.stats().await.file_count, OPEN_FILES as u64 + 1);
}

#[tokio::test(flavor = "current_thread")]
async fn evicting_10000_entries_does_not_delay_reads() {
    let dir = tempfile::tempdir().unwrap();
    write_prepared_dir(dir.path(), EVICTED_ENTRIES, "hot");
    let cache = Cache::open(dir.path(), limits(500, 0.5, EvictionStrategy::Lru));
    cache.ready().await;
    let (mut worst, mut reads) = (Duration::ZERO, 0);
    while cache.stats().await.evictions < EVICTED_ENTRIES as u64 - 30 {
        let before = Instant::now();
        assert!(cache.get("hot").await.is_some());
        worst = worst.max(before.elapsed());
        reads += 1;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(reads > 3, "eviction finished before reads overlapped: {reads}");
    assert!(worst < MAX_STALL, "a read waited {worst:?} during eviction");
    assert!(cache.contains("hot").await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn index_lock_hold_stays_short_with_100000_entries() {
    let dir = tempfile::tempdir().unwrap();
    write_prepared_dir(dir.path(), BIG_INDEX_ENTRIES, "hot");
    let cache = Cache::open(dir.path(), limits(u64::MAX / 2, 0.9, EvictionStrategy::Lru));
    cache.ready().await;
    cache.flush().await;
    let half = BIG_INDEX_ENTRIES as u64 * 10 / 2;
    cache.set_limits(limits(half, 0.9, EvictionStrategy::Lru)).await.unwrap();
    let stats = cache.stats().await;
    assert!(stats.evictions > 40_000, "eviction did not run: {}", stats.evictions);
    assert!(stats.max_hold > Duration::ZERO, "max_hold is not measured");
    assert!(stats.max_hold < MAX_HOLD, "index lock was held for {:?}", stats.max_hold);
}

#[tokio::test]
async fn diagnostics_count_open_writers() {
    let f = fixture(limits(u64::MAX / 2, 0.9, EvictionStrategy::Lru), &[]).await;
    let first = f.cache.begin_write("a", Kind::Image).await.unwrap();
    let second = f.cache.begin_write("b", Kind::Image).await.unwrap();
    assert_eq!(f.cache.stats().await.open_writers, 2);
    drop(first);
    drop(second);
    assert_eq!(f.cache.stats().await.open_writers, 0);
    assert_eq!(f.cache.stats().await.eviction_backlog, 0);
}
