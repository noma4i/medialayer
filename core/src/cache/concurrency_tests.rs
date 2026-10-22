use super::test_support::*;
use super::*;
use crate::limits::EvictionStrategy;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const MIB: usize = 1024 * 1024;
const TICK: Duration = Duration::from_millis(5);
const OPEN_FILES: usize = 20_000;
const EVICTED_ENTRIES: usize = 10_000;

fn spawn_ticker(stop: Arc<AtomicBool>) -> tokio::task::JoinHandle<u32> {
    tokio::spawn(async move {
        let mut ticks = 0;
        while !stop.load(Ordering::SeqCst) {
            tokio::time::sleep(TICK).await;
            ticks += 1;
        }
        ticks
    })
}

pub(super) fn write_prepared_dir(dir: &std::path::Path, files: usize, hot_key: &str) {
    use sha2::Digest;
    let checksum = super::entry::Check(vec![super::verify::sum_of(sha2::Sha256::new().chain_update(b"0123456789"))].into());
    let mut index = super::persist::HEADER.to_owned();
    for number in 0..files {
        let key = format!("entry-{number}");
        std::fs::write(dir.join(format!("{}.bin", crate::identity::key_file_name(&key))), b"0123456789").unwrap();
        let entry = super::entry::Entry { check: checksum.clone(), size: 10, kind: Kind::Image, created: number as u64, last_access: number as u64, hits: 0, meta: Default::default(), tone: None };
        super::persist::encode_line(&key, &entry, &mut index);
    }
    std::fs::write(dir.join(format!("{}.bin", crate::identity::key_file_name(hot_key))), b"0123456789").unwrap();
    let hot = super::entry::Entry { check: checksum.clone(), size: 10, kind: Kind::Image, created: 0, last_access: u64::MAX / 2, hits: 0, meta: Default::default(), tone: None };
    super::persist::encode_line(hot_key, &hot, &mut index);
    std::fs::write(dir.join("index.tsv"), index).unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn inv_cache_11_put_and_get_of_50_mib_do_not_stall_other_tasks() {
    let f = fixture(limits(u64::MAX / 2, 0.9, EvictionStrategy::Lru), &[]).await;
    let stop = Arc::new(AtomicBool::new(false));
    let ticker = spawn_ticker(stop.clone());
    let data = vec![7u8; 50 * MIB];
    f.cache.put("big", Kind::Stream, &data).await.unwrap();
    let read = f.cache.get("big").await.unwrap();
    stop.store(true, Ordering::SeqCst);
    let ticks = ticker.await.unwrap();
    assert_eq!(read.len(), data.len());
    assert!(ticks > 3, "other tasks barely ran during the put and the get: {ticks}");
}

#[tokio::test(flavor = "current_thread")]
async fn inv_cache_11_open_on_20000_files_returns_fast_and_reads_miss_until_ready() {
    let dir = tempfile::tempdir().unwrap();
    write_prepared_dir(dir.path(), OPEN_FILES, "hot");
    let cache = Cache::open(dir.path(), limits(u64::MAX / 2, 0.9, EvictionStrategy::Lru), u64::MAX);
    assert!(!cache.is_ready());
    assert_eq!(cache.get("hot").await, None);
    assert!(!cache.contains("hot").await);
    cache.ready().await;
    assert_eq!(cache.get("hot").await, Some(b"0123456789".to_vec()));
    assert_eq!(cache.stats().await.file_count, OPEN_FILES as u64 + 1);
}

#[tokio::test(flavor = "current_thread")]
async fn inv_cache_11_evicting_10000_entries_gives_other_tasks_a_turn_at_least_every_128_removals() {
    let dir = tempfile::tempdir().unwrap();
    write_prepared_dir(dir.path(), EVICTED_ENTRIES, "hot");
    let cache = Cache::open(dir.path(), limits(500, 0.5, EvictionStrategy::Lru), u64::MAX);
    cache.ready().await;
    let stop = Arc::new(AtomicBool::new(false));
    let turns = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let probe = {
        let (stop, turns) = (stop.clone(), turns.clone());
        tokio::spawn(async move {
            while !stop.load(Ordering::SeqCst) {
                turns.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
            }
        })
    };
    wait_until(|| !cache.inner.evicting.load(Ordering::SeqCst) && cache.inner.state().used <= 500).await;
    stop.store(true, Ordering::SeqCst);
    probe.await.unwrap();
    let evicted = cache.stats().await.evictions;
    assert!(evicted >= EVICTED_ENTRIES as u64 - 30, "eviction did not run: {evicted}");
    assert!(turns.load(Ordering::SeqCst) >= evicted / 128, "{} turns for {evicted} removals", turns.load(Ordering::SeqCst));
    assert!(cache.get("hot").await.is_some());
}

#[tokio::test]
async fn inv_cache_11_diagnostics_count_open_writers() {
    let f = fixture(limits(u64::MAX / 2, 0.9, EvictionStrategy::Lru), &[]).await;
    let first = f.cache.begin_write("a", Kind::Image).await.unwrap();
    let second = f.cache.begin_write("b", Kind::Image).await.unwrap();
    assert_eq!(f.cache.stats().await.open_writers, 2);
    drop(first);
    drop(second);
    assert_eq!(f.cache.stats().await.open_writers, 0);
    assert_eq!(f.cache.stats().await.eviction_backlog, 0);
}
