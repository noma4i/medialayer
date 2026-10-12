use super::runner_tests::{eventually, next, rig_with, video, Rig};
use super::*;
use crate::limits::{LeadingChunks, Limits, LimitsError, MIB};
use std::collections::{HashMap, HashSet};

const POLICY: [usize; 3] = [3, 2, 1];

fn master(item: usize) -> String {
    format!("https://o.test/v{item}/master.m3u8")
}

fn chunk(item: usize, index: usize) -> String {
    format!("https://o.test/v{item}/s{index}.ts")
}

fn playlist(chunks: usize, init: bool) -> String {
    let mut text = String::from("#EXTM3U\n");
    if init {
        text.push_str("#EXT-X-MAP:URI=\"init.mp4\"\n");
    }
    for index in 0..chunks {
        text.push_str(&format!("#EXTINF:3,\ns{index}.ts\n"));
    }
    text
}

fn feed(count: usize) -> Vec<Option<Source>> {
    (0..count).map(|item| Some(video(&master(item)))).collect()
}

fn rig_for(count: usize, chunks: usize, tweak: impl FnOnce(&mut Limits)) -> Rig {
    let rig = rig_with(|limits| {
        limits.prefetch_from = 1;
        limits.prefetch_to = 9;
        tweak(limits);
    });
    for item in 0..count {
        rig.loader.serve(&master(item), &playlist(chunks, false));
    }
    rig
}

fn requested(rig: &Rig, item: usize) -> usize {
    rig.loader.urls().iter().filter(|url| url.starts_with(&format!("https://o.test/v{item}/s"))).count()
}

fn wanted(distance: usize) -> usize {
    POLICY[distance.clamp(1, POLICY.len()) - 1]
}

async fn settle(rig: &Rig, events: &mut tokio::sync::broadcast::Receiver<Settled>, count: usize) {
    for _ in 0..count {
        next(events).await;
    }
    eventually(|| rig.prefetcher.live_tasks() == 0).await;
}

#[tokio::test]
async fn inv_prefetch_11_an_item_at_distance_d_gets_the_chunks_of_its_distance_and_never_fewer_than_one() {
    let rig = rig_for(10, 8, |_| {});
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.set_window(0, feed(10));
    settle(&rig, &mut events, 9).await;

    let counts: Vec<usize> = (1..10).map(|item| requested(&rig, item)).collect();
    assert_eq!(counts, [3, 2, 1, 1, 1, 1, 1, 1, 1]);
    assert_eq!(requested(&rig, 0), 0);
}

#[tokio::test]
async fn inv_prefetch_12_moving_the_window_requests_only_the_missing_chunks_and_never_a_stored_one() {
    let rig = rig_for(12, 8, |_| {});
    let mut events = rig.prefetcher.settled();
    let mut expected = vec![0usize; 12];
    for (step, active) in [0usize, 1, 4, 2, 7].into_iter().enumerate() {
        rig.prefetcher.set_window(active, feed(12));
        eventually(|| rig.prefetcher.live_tasks() == 0).await;
        for item in active + 1..(active + 10).min(12) {
            expected[item] = expected[item].max(wanted(item - active));
        }
        let urls: Vec<String> = rig.loader.urls().into_iter().filter(|url| url.contains("/s")).collect();
        assert_eq!(urls.len(), urls.iter().collect::<HashSet<_>>().len(), "step {step}: a chunk was requested twice");
        let counts: Vec<usize> = (0..12).map(|item| requested(&rig, item)).collect();
        assert_eq!(counts, expected, "step {step} active {active}");
    }
    while events.try_recv().is_ok() {}
}

#[tokio::test]
async fn inv_prefetch_13_when_the_distance_grows_the_deeper_request_is_cancelled_and_nothing_is_requested_or_deleted() {
    let rig = rig_for(6, 8, |_| {});
    rig.loader.hang(&chunk(3, 1));
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.set_window(2, feed(6));
    eventually(|| rig.loader.urls().contains(&chunk(3, 1))).await;
    let before = rig.loader.urls().len();
    let cancelled = rig.prefetcher.counters().cancelled;

    rig.prefetcher.set_window(0, feed(6));
    eventually(|| rig.loader.active() == 0 && rig.prefetcher.live_tasks() == 0).await;

    assert!(rig.prefetcher.counters().cancelled > cancelled);
    assert_eq!(requested(&rig, 3), 2);
    assert!(rig.loader.urls().len() >= before);
    assert!(rig.loader.evicted().is_empty());
    let mut loaded = HashMap::new();
    while let Ok(settled) = events.try_recv() {
        loaded.insert(settled.url, settled.outcome);
    }
    assert_eq!(loaded[&master(3)], Outcome::Loaded);
}

#[tokio::test]
async fn inv_prefetch_14_chunks_go_by_distance_first_and_chunk_index_second() {
    let rig = rig_for(4, 8, |limits| limits.max_concurrent = 1);
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.set_window(0, feed(4).into_iter().take(4).collect::<Vec<_>>());
    settle(&rig, &mut events, 3).await;

    let order: Vec<String> = rig.loader.urls();
    let expected = [
        master(1), chunk(1, 0), chunk(1, 1), chunk(1, 2),
        master(2), chunk(2, 0), chunk(2, 1),
        master(3), chunk(3, 0),
    ];
    assert_eq!(order, expected);
}

#[tokio::test]
async fn inv_prefetch_15_the_init_segment_loads_once_before_the_first_chunk_and_is_not_a_chunk() {
    let rig = rig_for(0, 0, |_| {});
    rig.loader.serve(&master(1), &playlist(5, true));
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.set_window(0, vec![None, Some(video(&master(1)))]);
    settle(&rig, &mut events, 1).await;
    rig.prefetcher.set_window(0, vec![None, Some(video(&master(1)))]);

    let init = "https://o.test/v1/init.mp4";
    assert_eq!(rig.loader.urls(), [master(1), init.to_string(), chunk(1, 0), chunk(1, 1), chunk(1, 2)]);
}

#[test]
fn inv_prefetch_16_an_invalid_chunk_list_or_budget_is_an_error_that_names_the_key() {
    let cases: [(&str, &str); 8] = [
        ("leading_chunks=", "leading_chunks"),
        ("leading_chunks=3,0,1", "leading_chunks"),
        ("leading_chunks=3,9", "leading_chunks"),
        ("leading_chunks=1,2", "leading_chunks"),
        ("leading_chunks=3,3,3,3,3,3,3,3,1", "leading_chunks"),
        ("leading_chunks=a", "leading_chunks"),
        ("prefetch_budget_mib=0", "prefetch_budget_mib"),
        ("prefetch_budget_mib=513", "prefetch_budget_mib"),
    ];
    for (text, key) in cases {
        let error = Limits::decode(Limits::default(), text).expect_err(text);
        assert!(error.to_string().contains(key), "{text}: {error}");
    }
    let limits = Limits::decode(Limits::default(), "leading_chunks=4, 2,1\nprefetch_budget_mib=512").unwrap();
    assert_eq!(limits.leading_chunks.to_vec(), [4, 2, 1]);
    assert_eq!(LeadingChunks::new(&[]), Err(LimitsError::LeadingChunks { reason: "must not be empty" }));
}

fn held(ledger: &mut PrefetchLedger, key: &str, distance: usize, chunk: usize, bytes: u64, budget: u64) -> (Vec<Vec<String>>, bool) {
    let target = chunk + 1;
    ledger.needed(key, target, std::time::Duration::from_secs(1), true);
    ledger.set_distance(key, distance);
    let plan = super::plan::Plan {
        chunks: (0..8).map(|index| crate::hls::Chunk { segment: format!("{key}/{index}"), init: None }).collect(),
        headers: HashMap::new(),
    };
    ledger.set_plan(key, plan);
    let (evicted, itself) = ledger.record_chunk(key, chunk, vec![(format!("{key}/{chunk}"), bytes)], None, budget);
    assert!(ledger.held_bytes() <= budget, "{key} chunk {chunk}: {} bytes held over {budget}", ledger.held_bytes());
    (evicted.into_iter().map(|entry| entry.urls).collect(), itself)
}

#[test]
fn inv_prefetch_17_held_bytes_stay_within_the_budget_after_every_chunk_and_the_farthest_goes_first() {
    let budget = 2 * MIB;
    let mut ledger = PrefetchLedger::new(std::time::Duration::from_secs(60), 100);
    assert_eq!(held(&mut ledger, "far", 3, 0, MIB, budget), (vec![], false));
    assert_eq!(held(&mut ledger, "near", 1, 0, MIB, budget), (vec![], false));
    let (evicted, itself) = held(&mut ledger, "near", 1, 1, MIB, budget);
    assert_eq!((evicted, itself), (vec![vec!["far/0".to_string()]], false));
    let (evicted, itself) = held(&mut ledger, "middle", 2, 0, MIB, budget);
    assert_eq!((evicted, itself), (vec![vec!["middle/0".to_string()]], true));
    assert_eq!(ledger.held_bytes(), 2 * MIB);
}

#[tokio::test]
async fn inv_prefetch_17_a_chunk_that_would_be_the_farthest_over_the_budget_is_not_fetched() {
    let rig = rig_for(4, 8, |limits| {
        limits.max_concurrent = 1;
        limits.prefetch_budget_mib = 2;
    });
    for item in 1..4 {
        for index in 0..8 {
            rig.loader.size(&chunk(item, index), MIB);
        }
    }
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.set_window(0, feed(4));
    let mut outcomes = HashMap::new();
    for _ in 0..3 {
        let settled = next(&mut events).await;
        outcomes.insert(settled.url, settled.outcome);
    }

    assert_eq!(outcomes[&master(1)], Outcome::OverBudget);
    assert_eq!(outcomes[&master(2)], Outcome::OverBudget);
    assert_eq!(outcomes[&master(3)], Outcome::OverBudget);
    let urls = rig.loader.urls();
    assert!(urls.contains(&chunk(1, 1)) && !urls.contains(&chunk(1, 2)) && !urls.contains(&chunk(2, 0)) && !urls.contains(&chunk(3, 0)));
    assert!(rig.prefetcher.held_bytes() <= 2 * MIB);
}

#[tokio::test]
async fn inv_prefetch_18_a_playlist_shorter_than_the_policy_loads_all_its_chunks_without_error() {
    let rig = rig_for(0, 0, |_| {});
    rig.loader.serve(&master(1), &playlist(2, false));
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.set_window(0, vec![None, Some(video(&master(1)))]);

    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    assert_eq!(rig.loader.urls(), [master(1), chunk(1, 0), chunk(1, 1)]);
}

#[tokio::test]
async fn inv_prefetch_19_after_the_loads_settle_the_prefetcher_holds_no_task_and_the_same_window_asks_for_nothing() {
    let rig = rig_for(10, 8, |_| {});
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.set_window(0, feed(10));
    settle(&rig, &mut events, 9).await;
    let calls = rig.loader.urls().len();

    for _ in 0..3 {
        rig.prefetcher.set_window(0, feed(10));
    }
    eventually(|| rig.prefetcher.live_tasks() == 0).await;

    assert_eq!(rig.loader.urls().len(), calls);
    assert_eq!(rig.prefetcher.live_tasks(), 0);
}
