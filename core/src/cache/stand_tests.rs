use super::test_support::*;
use super::*;
use crate::limits::EvictionStrategy;
use crate::seeded::Dice;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

const LANES: u64 = 8;
const OPERATIONS: usize = 300;
const KEYS: usize = 6;
const BUDGET: u64 = 30_000;
const MAX_VALUE: usize = 9_000;

fn key_of(index: usize) -> String {
    format!("https://o.test/stand/{index}")
}

fn checksum(body: &[u8]) -> u64 {
    body.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3))
}

fn sealed(dice: &mut Dice) -> Vec<u8> {
    let len = dice.index(MAX_VALUE);
    let body = dice.bytes(len);
    let mut value = checksum(&body).to_le_bytes().to_vec();
    value.extend_from_slice(&body);
    value
}

fn is_whole(value: &[u8]) -> bool {
    value.len() >= 8 && value[..8] == checksum(&value[8..]).to_le_bytes()
}

fn held(cache: &Cache) -> (u64, u64) {
    let state = cache.inner.state();
    (state.used + state.reserved, state.limits.budget)
}

async fn lane(cache: Cache, mut dice: Dice, vary_budget: bool, torn: Arc<AtomicU64>, refused: Arc<AtomicU64>) {
    for _ in 0..OPERATIONS {
        let key = key_of(dice.index(KEYS));
        match dice.below(11) {
            0..=2 => {
                let value = sealed(&mut dice);
                match cache.put(&key, Kind::Stream, &value).await {
                    Ok(()) => {}
                    Err(CacheError::NotEnoughSpace) => {
                        refused.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(error) => panic!("put failed with {error}"),
                }
            }
            3 | 4 => {
                if let Some(value) = cache.get(&key).await {
                    if !is_whole(&value) {
                        torn.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
            5 => {
                if let Some(mut reader) = cache.open_reader(&key).await {
                    let mut value = Vec::new();
                    loop {
                        tokio::task::yield_now().await;
                        let chunk = reader.read_chunk().await.unwrap();
                        if chunk.is_empty() {
                            break;
                        }
                        value.extend_from_slice(&chunk);
                    }
                    if !is_whole(&value) {
                        torn.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
            6 | 7 => {
                let value = sealed(&mut dice);
                let Ok(mut writer) = cache.begin_write(&key, Kind::Image).await else { continue };
                let mut ending = dice.below(4);
                let cut = if ending == 3 { dice.index(3) } else { usize::MAX };
                for (position, chunk) in value.chunks(700).enumerate() {
                    if position == cut {
                        break;
                    }
                    if writer.write(chunk).await.is_err() {
                        ending = 1;
                        break;
                    }
                    tokio::task::yield_now().await;
                }
                match ending {
                    0 => {
                        let _ = writer.commit().await;
                    }
                    1 => writer.abort().await,
                    _ => drop(writer),
                }
            }
            8 => {
                let budget = Duration::from_micros(dice.between(1, 400));
                let value = sealed(&mut dice);
                let _ = tokio::time::timeout(budget, cache.put(&key, Kind::Audio, &value)).await;
            }
            9 => {
                match dice.below(4) {
                    0 => {
                        cache.remove(&key).await;
                    }
                    1 => {
                        cache.purge_prefix("https://o.test/stand/").await;
                    }
                    2 => {
                        cache.clear(Some(Kind::Image)).await;
                    }
                    _ => cache.evict_now().await,
                }
            }
            _ => {
                if vary_budget {
                    let budget = if dice.percent(50) { BUDGET } else { BUDGET / 2 };
                    cache.set_limits(limits(budget, 0.6, EvictionStrategy::Lru)).await.unwrap();
                }
            }
        }
    }
}

async fn run(name: &'static str, vary_budget: bool) {
    let dice = Dice::new(name);
    let f = fixture(limits(BUDGET, 0.6, EvictionStrategy::Lru), &[]).await;
    let torn = Arc::new(AtomicU64::new(0));
    let refused = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (cache, stop) = (f.cache.clone(), stop.clone());
        tokio::spawn(async move {
            let mut worst = 0u64;
            while !stop.load(Ordering::SeqCst) {
                let (total, budget) = held(&cache);
                if !vary_budget {
                    assert!(total <= budget, "{total} bytes used or reserved over the budget {budget}");
                }
                worst = worst.max(total);
                tokio::task::yield_now().await;
            }
            worst
        })
    };
    let lanes: Vec<_> =
        (0..LANES).map(|index| tokio::spawn(lane(f.cache.clone(), dice.fork(index), vary_budget, torn.clone(), refused.clone()))).collect();
    for lane in lanes {
        lane.await.unwrap();
    }
    stop.store(true, Ordering::SeqCst);
    let worst = watcher.await.unwrap();
    assert!(worst > 0, "the watcher never saw any data");
    assert_eq!(torn.load(Ordering::SeqCst), 0, "a reader saw bytes that were never committed whole");
    assert!(f.cache.stats().await.evictions > 0 || refused.load(Ordering::SeqCst) > 0, "the budget never mattered, the stand is too weak");

    wait_until(|| {
        let state = f.cache.inner.state();
        state.reserved == 0 && state.live_parts.is_empty() && !f.cache.inner.evicting.load(Ordering::SeqCst)
    })
    .await;
    f.cache.evict_now().await;
    let (total, budget) = held(&f.cache);
    assert!(total <= budget, "settled cache holds {total} over the budget {budget}");
    let stats = f.cache.stats().await;
    assert_eq!(stats.open_writers, 0);
    let mut bytes = 0;
    let mut files = 0;
    for index in 0..KEYS {
        if let Some(value) = f.cache.get(&key_of(index)).await {
            assert!(is_whole(&value), "settled value of key {index} is torn");
            bytes += value.len() as u64;
            files += 1;
        }
    }
    assert_eq!((stats.file_count, stats.total_bytes), (files, bytes), "books after the stand");
    wait_until(|| files_with_suffix(f.dir.path(), ".part") == 0 && files_with_suffix(f.dir.path(), ".bin") as u64 == files).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inv_cache_01_inv_cache_02_writers_readers_and_cancels_never_pass_the_budget_or_leak() {
    run("inv_cache_01_inv_cache_02_writers_readers_and_cancels_never_pass_the_budget_or_leak", false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inv_cache_10_limits_changing_under_load_settle_to_a_balanced_cache_within_the_last_budget() {
    run("inv_cache_10_limits_changing_under_load_settle_to_a_balanced_cache_within_the_last_budget", true).await;
}
