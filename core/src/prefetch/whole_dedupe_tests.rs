use super::loader::ProxyLoader;
use super::runner_tests::{eventually, next, rig, rig_with, Rig};
use super::whole_tests::{stack, Stack};
use super::*;
use crate::cache::{Clock, ManualClock};
use crate::fetch::mock::Script;
use crate::limits::Limits;
use crate::proxy::Priority;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const URL: &str = "https://o.test/a.jpg";

fn photo(strategy: CacheStrategy) -> Source {
    Source { url: URL.to_string(), headers: HashMap::new(), kind: SourceKind::Image, strategy }
}

fn fetches(rig: &Rig) -> usize {
    rig.loader.urls().iter().filter(|url| *url == URL).count()
}

pub(super) fn over_proxy(stack: &Stack) -> Prefetcher {
    let limits = Limits { prefetch_from: 0, prefetch_to: 0, ..Limits::default() };
    let ledger = PrefetchLedger::new(Duration::from_secs(limits.dedupe_window_seconds.into()), limits.dedupe_max_entries as usize);
    Prefetcher::new(Arc::new(ProxyLoader::new(stack.proxy.clone())), &limits, ledger, Arc::new(ManualClock::new(1_000)) as Arc<dyn Clock>)
}

#[tokio::test(start_paused = true)]
async fn inv_strategy_22_a_failing_whole_source_is_fetched_three_times_in_one_block_and_a_recovering_one_twice() {
    let failing = rig();
    let mut events = failing.prefetcher.settled();
    failing.loader.fail(URL, true);
    failing.prefetcher.keep_whole(photo(CacheStrategy::Whole));
    assert!(matches!(next(&mut events).await.outcome, Outcome::Failed(_)));
    tokio::time::advance(Duration::from_secs(120)).await;
    assert_eq!(failing.prefetcher.live_tasks(), 0);
    assert_eq!(fetches(&failing), 3);

    let recovering = rig();
    let mut events = recovering.prefetcher.settled();
    recovering.loader.fail(URL, true);
    recovering.loader.hang(URL);
    recovering.prefetcher.keep_whole(photo(CacheStrategy::Whole));
    eventually(|| fetches(&recovering) == 1).await;
    recovering.loader.fail(URL, false);
    recovering.loader.release(URL);
    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    assert_eq!(fetches(&recovering), 2);
}

#[tokio::test]
async fn inv_strategy_23_suspend_and_resume_restart_a_whole_load_once_and_resume_or_foreground_alone_add_none() {
    let rig = rig();
    rig.loader.hang(URL);
    rig.prefetcher.keep_whole(photo(CacheStrategy::Whole));
    eventually(|| rig.loader.alive() == [URL]).await;

    rig.prefetcher.set_foreground_busy(true);
    rig.prefetcher.set_foreground_busy(false);
    rig.prefetcher.resume();
    assert_eq!(rig.prefetcher.live_tasks(), 1);
    assert_eq!(fetches(&rig), 1);

    rig.prefetcher.suspend();
    eventually(|| rig.loader.alive().is_empty()).await;
    rig.prefetcher.resume();
    eventually(|| rig.loader.alive() == [URL]).await;
    assert_eq!(rig.prefetcher.live_tasks(), 1);
    assert_eq!((fetches(&rig), rig.loader.alive().len()), (2, 1));
}

#[tokio::test]
async fn inv_strategy_24_a_window_and_a_whole_declaration_of_one_url_share_the_running_load_in_both_orders() {
    for whole_first in [true, false] {
        let rig = rig_with(|limits| {
            limits.prefetch_from = 0;
            limits.prefetch_to = 0;
        });
        rig.loader.hang(URL);
        let window = |rig: &Rig| rig.prefetcher.set_window(0, vec![Some(photo(CacheStrategy::Window))]);
        let whole = |rig: &Rig| rig.prefetcher.keep_whole(photo(CacheStrategy::Whole));
        if whole_first { whole(&rig) } else { window(&rig) }
        eventually(|| rig.loader.alive() == [URL]).await;
        if whole_first { window(&rig) } else { whole(&rig) }
        assert_eq!(rig.prefetcher.live_tasks(), 1);
        assert_eq!((fetches(&rig), rig.loader.alive().len()), (1, 1), "whole first: {whole_first}");
    }
}

#[tokio::test]
async fn inv_strategy_25_a_window_load_then_a_whole_declaration_reach_the_network_once_in_both_orders() {
    for whole_first in [true, false] {
        let stack = stack().await;
        stack.fetcher.route(URL, Script::ok(vec![7; 2048]));
        let prefetcher = over_proxy(&stack);
        let mut events = prefetcher.settled();
        let window = || prefetcher.set_window(0, vec![Some(photo(CacheStrategy::Window))]);
        let whole = || prefetcher.keep_whole(photo(CacheStrategy::Whole));
        if whole_first { whole() } else { window() }
        assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
        if whole_first { window() } else { whole() }
        if prefetcher.live_tasks() > 0 {
            assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
        }
        assert_eq!(stack.fetcher.calls_for(URL), 1, "whole first: {whole_first}");
    }
}

#[tokio::test]
async fn inv_strategy_26_a_declaration_made_right_after_a_release_leaves_exactly_one_live_fetch() {
    let rig = rig();
    rig.loader.hang(URL);
    rig.prefetcher.hold_whole(1, photo(CacheStrategy::Whole));
    eventually(|| rig.loader.alive() == [URL]).await;

    rig.prefetcher.release_owner(1);
    rig.prefetcher.hold_whole(2, photo(CacheStrategy::Whole));
    eventually(|| fetches(&rig) == 2 && rig.loader.alive().len() == 1).await;
    assert_eq!(rig.prefetcher.live_tasks(), 1);
    assert_eq!((fetches(&rig), rig.loader.alive().len(), rig.prefetcher.counters().cancelled), (2, 1, 1));
}

#[tokio::test]
async fn inv_strategy_27_an_image_load_of_a_place_and_a_whole_load_of_the_same_url_share_one_upstream_fetch() {
    let stack = stack().await;
    stack.fetcher.route(URL, Script::ok(vec![7; 2048]).head_delay(Duration::from_millis(150)));
    let loader = ProxyLoader::new(stack.proxy.clone());
    let headers = HashMap::new();
    let progress = tokio::sync::Notify::new();

    let (place, whole) = tokio::join!(
        stack.proxy.fetch_bytes(URL, &headers, u64::MAX),
        super::loader::Loader::load(&loader, URL, &headers, Priority::ImagePrefetch, &progress),
    );

    assert!(place.is_ok() && whole.is_ok());
    assert_eq!(stack.fetcher.calls_for(URL), 1);
}

#[tokio::test]
async fn inv_strategy_28_forgetting_a_loaded_source_allows_exactly_one_new_load_and_nothing_else_does() {
    let rig = rig();
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.keep_whole(photo(CacheStrategy::Whole));
    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    rig.prefetcher.keep_whole(photo(CacheStrategy::Whole));
    assert_eq!(rig.prefetcher.live_tasks(), 0);
    assert_eq!(fetches(&rig), 1);

    rig.prefetcher.forget_loaded(Some(URL));
    rig.prefetcher.keep_whole(photo(CacheStrategy::Whole));
    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    assert_eq!(rig.prefetcher.live_tasks(), 0);
    assert_eq!(fetches(&rig), 2);
}

fn item(index: usize) -> Source {
    Source { url: format!("https://o.test/w{index}.jpg"), headers: HashMap::new(), kind: SourceKind::Image, strategy: CacheStrategy::Whole }
}

#[tokio::test]
async fn inv_strategy_32_a_shown_place_is_never_refused_by_the_whole_entry_limit() {
    let rig = rig_with(|limits| limits.dedupe_max_entries = 2);
    for index in (0..6).chain(std::iter::once(9)) {
        rig.loader.hang(&item(index).url);
    }
    let mut events = rig.prefetcher.settled();
    for index in 0..6 {
        rig.prefetcher.keep_shown_whole(item(index));
    }
    assert_eq!(rig.prefetcher.counters().failed, 0);
    assert!(events.try_recv().is_err(), "a shown place got a settled outcome while its load is pending");
    rig.prefetcher.hold_whole(1, item(9));
    assert_eq!(rig.prefetcher.counters().failed, 1, "a preload hold past the limit is still one refusal");
    assert!(matches!(next(&mut events).await.outcome, Outcome::Failed(_)));
    for index in 0..6 {
        rig.loader.release(&item(index).url);
    }
    for _ in 0..6 {
        assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    }
}

#[tokio::test]
async fn inv_strategy_32_random_holds_refuse_only_preload_past_the_limit() {
    let mut dice = crate::seeded::Dice::new("inv_strategy_32_random_holds_refuse_only_preload_past_the_limit");
    for _ in 0..20 {
        let limit = dice.between(1, 4) as usize;
        let rig = rig_with(|limits| limits.dedupe_max_entries = limit as u32);
        rig.loader.hang_all();
        let mut held: HashMap<usize, std::collections::HashSet<u64>> = HashMap::new();
        let mut refused = 0u64;
        for _ in 0..60 {
            let index = dice.index(8);
            match dice.below(3) {
                0 => {
                    rig.prefetcher.keep_shown_whole(item(index));
                    held.entry(index).or_default().insert(0);
                }
                1 => {
                    let owner = dice.between(1, 3);
                    rig.prefetcher.hold_whole(owner, item(index));
                    if !held.contains_key(&index) && held.len() >= limit {
                        refused += 1;
                    } else {
                        held.entry(index).or_default().insert(owner);
                    }
                }
                _ => {
                    let owner = dice.between(1, 3);
                    rig.prefetcher.release_owner(owner);
                    held.retain(|_, owners| {
                        owners.remove(&owner);
                        !owners.is_empty()
                    });
                }
            }
        }
        assert_eq!(rig.prefetcher.counters().failed, refused, "limit {limit}");
    }
}
