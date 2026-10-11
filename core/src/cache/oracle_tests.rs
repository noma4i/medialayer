use super::test_support::*;
use super::*;
use crate::limits::EvictionStrategy;
use crate::proxy::Meta;
use crate::seeded::Dice;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

const KEY_COUNT: usize = 10;
const MAX_LEN: u64 = 8_000;
const BIG_LEN: usize = 1_300_000;
const STRATEGIES: [EvictionStrategy; 5] =
    [EvictionStrategy::Lru, EvictionStrategy::Fifo, EvictionStrategy::Lfu, EvictionStrategy::LargestFirst, EvictionStrategy::ExpireAfter(1)];

type Model = HashMap<String, (Kind, Vec<u8>)>;

fn key_of(index: usize) -> String {
    format!("https://o.test/{}/{index}", if index.is_multiple_of(2) { "even" } else { "odd" })
}

fn kind_of(dice: &mut Dice) -> Kind {
    Kind::ALL[dice.index(3)]
}

fn value(dice: &mut Dice, rare_big: bool) -> Vec<u8> {
    let len = if rare_big && dice.percent(1) { BIG_LEN } else { dice.below(MAX_LEN) as usize };
    dice.bytes(len)
}

fn used_plus_reserved(cache: &Cache) -> (u64, u64) {
    let state = cache.inner.state();
    (state.used + state.reserved, state.limits.budget)
}

async fn settle(cache: &Cache) {
    wait_until(|| !cache.inner.evicting.load(std::sync::atomic::Ordering::SeqCst) && cache.inner.state().reserved == 0).await;
}

async fn quiet_disk(dir: &Path) {
    wait_until(|| files_with_suffix(dir, ".part") == 0).await;
}

async fn expect_exact(cache: &Cache, model: &Model) {
    for index in 0..KEY_COUNT {
        let key = key_of(index);
        let actual = cache.get(&key).await;
        match model.get(&key) {
            Some((_, bytes)) => {
                assert_eq!(actual.as_deref(), Some(bytes.as_slice()), "{key} differs from the model");
                assert_eq!(cache.len(&key).await, Some(bytes.len() as u64), "{key} length");
            }
            None => assert_eq!(actual, None, "{key} is in the cache but not in the model"),
        }
    }
}

async fn expect_stats(cache: &Cache, model: &Model) {
    let stats = cache.stats().await;
    assert_eq!(stats.file_count, model.len() as u64, "file count");
    assert_eq!(stats.total_bytes, model.values().map(|(_, bytes)| bytes.len() as u64).sum::<u64>(), "total bytes");
    for kind in Kind::ALL {
        let of_kind = model.values().filter(|(model_kind, _)| *model_kind == kind);
        let expected = KindStats { bytes: of_kind.clone().map(|(_, bytes)| bytes.len() as u64).sum(), files: of_kind.count() as u64 };
        assert_eq!(stats.per_kind[&kind], expected, "{kind:?} stats");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inv_cache_05_the_cache_equals_a_map_model_over_random_operations() {
    let mut dice = Dice::new("inv_cache_05_the_cache_equals_a_map_model_over_random_operations");
    let roomy = limits(1 << 40, 0.9, EvictionStrategy::Lru);
    let mut f = fixture(roomy, &[]).await;
    let mut model: Model = HashMap::new();
    for step in 0..500 {
        let index = dice.index(KEY_COUNT);
        let key = key_of(index);
        match dice.below(13) {
            0..=2 => {
                let (kind, bytes) = (kind_of(&mut dice), value(&mut dice, true));
                f.cache.put(&key, kind, &bytes).await.unwrap();
                model.insert(key, (kind, bytes));
            }
            3 => {
                let expected = model.get(&key).map(|(_, bytes)| bytes.clone());
                let start = dice.below(MAX_LEN);
                let len = dice.below(MAX_LEN);
                let slice = expected.as_ref().map(|bytes| {
                    let from = (start as usize).min(bytes.len());
                    bytes[from..(from + len as usize).min(bytes.len())].to_vec()
                });
                assert_eq!(f.cache.get_range(&key, start, len).await, slice, "range {start}+{len} of {key}");
            }
            4 => assert_eq!(f.cache.remove(&key).await, model.remove(&key).is_some(), "remove {key}"),
            5 => {
                let prefix = if dice.percent(50) { "https://o.test/even/" } else { "https://o.test/odd/" };
                let expected = model.keys().filter(|key| key.starts_with(prefix)).count();
                assert_eq!(f.cache.purge_prefix(prefix).await, expected, "purge {prefix}");
                model.retain(|key, _| !key.starts_with(prefix));
            }
            6 if dice.percent(30) => {
                let kind = dice.percent(60).then(|| kind_of(&mut dice));
                let expected = model.values().filter(|(model_kind, _)| kind.is_none_or(|wanted| wanted == *model_kind)).count();
                assert_eq!(f.cache.clear(kind).await, expected, "clear {kind:?}");
                model.retain(|_, (model_kind, _)| kind.is_some_and(|wanted| wanted != *model_kind));
            }
            7 | 8 => {
                let (kind, bytes) = (kind_of(&mut dice), value(&mut dice, false));
                let meta = Meta { content_type: Some(format!("t/{}", dice.below(5))), total_length: Some(bytes.len() as u64 + 7) };
                let mut writer = f.cache.begin_write(&key, kind).await.unwrap().with_meta(meta.clone());
                let mut at = 0;
                while at < bytes.len() {
                    let end = (at + 1 + dice.index(3_000)).min(bytes.len());
                    writer.write(&bytes[at..end]).await.unwrap();
                    at = end;
                }
                writer.commit().await.unwrap();
                assert_eq!(f.cache.meta(&key).await, Some(meta));
                model.insert(key, (kind, bytes));
            }
            9 => {
                let mut writer = f.cache.begin_write(&key, Kind::Stream).await.unwrap();
                writer.write(&dice.bytes(500)).await.unwrap();
                if dice.percent(50) {
                    writer.abort().await;
                } else {
                    drop(writer);
                }
            }
            10 => {
                assert_eq!(f.cache.contains(&key).await, model.contains_key(&key));
                assert_eq!(f.cache.len(&key).await, model.get(&key).map(|(_, bytes)| bytes.len() as u64));
            }
            11 if dice.percent(8) => {
                settle(&f.cache).await;
                quiet_disk(f.dir.path()).await;
                f.cache.flush().await;
                f.cache = Cache::open_with(f.dir.path(), roomy, options(f.clock.clone(), &[]));
                f.cache.ready().await;
            }
            _ => {
                f.clock.advance_ms(dice.below(50));
            }
        }
        settle(&f.cache).await;
        expect_exact(&f.cache, &model).await;
        expect_stats(&f.cache, &model).await;
        let (held, budget) = used_plus_reserved(&f.cache);
        assert!(held <= budget, "step {step}: {held} bytes held over the budget {budget}");
    }
    quiet_disk(f.dir.path()).await;
    assert_eq!(files_with_suffix(f.dir.path(), ".bin"), model.len());
    assert_eq!(f.cache.stats().await.open_writers, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inv_cache_06_under_eviction_a_read_is_a_miss_or_the_exact_value_and_the_books_balance() {
    for (round, strategy) in STRATEGIES.into_iter().enumerate() {
        let mut dice = Dice::new("inv_cache_06_under_eviction_a_read_is_a_miss_or_the_exact_value_and_the_books_balance").fork(round as u64);
        let budget = 20_000;
        let f = fixture(limits(budget, 0.5, strategy), &[]).await;
        let mut model: Model = HashMap::new();
        let mut refusals = 0;
        for step in 0..400 {
            let key = key_of(dice.index(KEY_COUNT));
            f.clock.advance_ms(dice.below(40));
            match dice.below(6) {
                0..=3 => {
                    let (kind, bytes) = (kind_of(&mut dice), value(&mut dice, false));
                    let previous = model.get(&key).map_or(0, |(_, old)| old.len() as u64);
                    match f.cache.put(&key, kind, &bytes).await {
                        Ok(()) => {
                            model.insert(key, (kind, bytes));
                        }
                        Err(error) => {
                            refusals += 1;
                            assert_eq!(error, CacheError::NotEnoughSpace);
                            assert!(
                                bytes.len() as u64 > budget || previous + bytes.len() as u64 > budget,
                                "step {step}: a put of {} bytes was refused with {previous} bytes held by its own key",
                                bytes.len()
                            );
                        }
                    }
                }
                4 => {
                    f.cache.remove(&key).await;
                    model.remove(&key);
                }
                _ => {
                    f.cache.evict_now().await;
                }
            }
            let (held, limit) = used_plus_reserved(&f.cache);
            assert!(held <= limit, "step {step}: {held} bytes held over the budget {limit}");
            settle(&f.cache).await;
            for index in 0..KEY_COUNT {
                let key = key_of(index);
                match (f.cache.get(&key).await, model.get(&key)) {
                    (Some(actual), Some((_, expected))) => assert_eq!(&actual, expected, "step {step}: {key} returned other bytes"),
                    (Some(_), None) => panic!("step {step}: {key} came back although it was removed or never stored"),
                    (None, _) => {
                        model.remove(&key);
                    }
                }
            }
        }
        settle(&f.cache).await;
        let stats = f.cache.stats().await;
        assert!(stats.evictions > 0, "{strategy:?}: the run never evicted, the budget is too large");
        assert!(refusals < 400, "{strategy:?}: every put was refused");
        for index in 0..KEY_COUNT {
            if f.cache.get(&key_of(index)).await.is_none() {
                model.remove(&key_of(index));
            }
        }
        expect_stats(&f.cache, &model).await;
        quiet_disk(f.dir.path()).await;
        wait_until(|| files_with_suffix(f.dir.path(), ".bin") == model.len()).await;
    }
}

fn snapshot(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let _ = std::fs::copy(entry.path(), to.join(entry.file_name()));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inv_cache_07_a_directory_copied_at_any_moment_opens_to_a_consistent_cache() {
    let mut dice = Dice::new("inv_cache_07_a_directory_copied_at_any_moment_opens_to_a_consistent_cache");
    let f = fixture(limits(1 << 40, 0.9, EvictionStrategy::Lru), &[]).await;
    let mut history: HashMap<String, Vec<Vec<u8>>> = HashMap::new();
    let mut copies = Vec::new();
    for step in 0..120 {
        let key = key_of(dice.index(KEY_COUNT));
        match dice.below(7) {
            0..=3 => {
                let bytes = value(&mut dice, false);
                history.entry(key.clone()).or_default().push(bytes.clone());
                if dice.percent(50) {
                    f.cache.put(&key, Kind::Stream, &bytes).await.unwrap();
                } else {
                    let mut writer = f.cache.begin_write(&key, Kind::Image).await.unwrap();
                    for chunk in bytes.chunks(900) {
                        writer.write(chunk).await.unwrap();
                        if dice.percent(10) {
                            let copy = tempfile::tempdir().unwrap();
                            snapshot(f.dir.path(), copy.path());
                            copies.push(copy);
                        }
                    }
                    writer.commit().await.unwrap();
                }
            }
            4 => {
                f.cache.remove(&key).await;
            }
            5 => f.cache.flush().await,
            _ => tokio::time::sleep(Duration::from_millis(dice.between(0, 25))).await,
        }
        if step % 6 == 0 {
            let copy = tempfile::tempdir().unwrap();
            snapshot(f.dir.path(), copy.path());
            copies.push(copy);
        }
    }
    assert!(copies.len() > 20, "too few snapshots: {}", copies.len());
    let big = limits(1 << 40, 0.9, EvictionStrategy::Lru);
    for (number, copy) in copies.iter().enumerate() {
        let reopened = Cache::open(copy.path(), big);
        reopened.ready().await;
        let mut total = 0;
        let mut present = 0;
        for index in 0..KEY_COUNT {
            let key = key_of(index);
            if let Some(bytes) = reopened.get(&key).await {
                present += 1;
                total += bytes.len() as u64;
                let known = history.get(&key).is_some_and(|values| values.contains(&bytes));
                assert!(known, "snapshot {number}: {key} holds {} bytes that were never committed", bytes.len());
                assert_eq!(reopened.len(&key).await, Some(bytes.len() as u64), "snapshot {number}: length of {key}");
            }
        }
        let stats = reopened.stats().await;
        assert_eq!((stats.file_count, stats.total_bytes), (present, total), "snapshot {number}: books");
        wait_until(|| files_with_suffix(copy.path(), ".part") == 0 && files_with_suffix(copy.path(), ".bin") as u64 == present).await;
    }
}

#[tokio::test]
async fn inv_cache_07_a_rewrite_on_disk_before_its_index_is_not_trusted_and_a_torn_index_tmp_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let slow_index = Options { persist_delay: Duration::from_secs(3600), ..Options::default() };
    let roomy = limits(1 << 30, 0.9, EvictionStrategy::Lru);
    let first = Cache::open_with(dir.path(), roomy, slow_index.clone());
    first.ready().await;
    first.put("https://o.test/k", Kind::Image, &[1u8; 100]).await.unwrap();
    first.flush().await;
    first.put("https://o.test/k", Kind::Image, &[2u8; 250]).await.unwrap();
    first.put("https://o.test/only-on-disk", Kind::Image, &[3u8; 40]).await.unwrap();
    std::fs::write(dir.path().join("index.tsv.tmp"), b"torn\tline without fields").unwrap();

    let reopened = Cache::open(dir.path(), roomy);
    reopened.ready().await;
    assert_eq!(reopened.get("https://o.test/k").await, None, "a file that disagrees with its index is dropped and fetched again");
    assert_eq!(reopened.len("https://o.test/k").await, None);
    assert!(!reopened.contains("https://o.test/only-on-disk").await, "a file without an index line is an orphan");
    let stats = reopened.stats().await;
    assert_eq!((stats.file_count, stats.total_bytes), (0, 0));
    reopened.put("https://o.test/k", Kind::Image, &[2u8; 250]).await.unwrap();
    reopened.flush().await;
    let again = Cache::open(dir.path(), roomy);
    again.ready().await;
    assert_eq!(again.get("https://o.test/k").await, Some(vec![2u8; 250]));
}
