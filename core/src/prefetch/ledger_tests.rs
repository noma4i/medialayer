use super::*;
use crate::hls::Chunk;
use crate::prefetch::plan::Plan;
use std::collections::HashMap;
use std::time::Duration;

fn secs(value: u64) -> Duration {
    Duration::from_secs(value)
}

fn ledger() -> PrefetchLedger {
    PrefetchLedger::new(secs(60), 500)
}

#[test]
fn inv_cache_28_purge_keeps_other_sources_loaded() {
    let mut ledger = ledger();
    for key in ["a", "a#ns:1", "b"] {
        ledger.needed(key, 1, secs(100), true);
        ledger.set_plan(key, plan(1));
        ledger.record_chunk(key, 0, vec![(format!("{key}/chunk"), 10)], None, u64::MAX);
    }
    ledger.forget_loaded(Some("a"));
    assert_eq!(ledger.step("a", 1), super::ledger::NextLoad::NeedPlan);
    assert_eq!(ledger.step("a#ns:1", 1), super::ledger::NextLoad::NeedPlan);
    assert_eq!(ledger.step("b", 1), super::ledger::NextLoad::Done);
    assert_eq!(ledger.held_bytes(), 10);
}

#[test]
fn inv_strategy_12_reset_and_expiry_discard_the_hls_plan_and_accounting() {
    for expire in [false, true] {
        let mut ledger = ledger();
        assert!(ledger.needed("a", 1, secs(100), true));
        ledger.set_plan("a", plan(3));
        ledger.record_chunk("a", 0, vec![("c0".into(), 10)], None, u64::MAX);
        if expire {
            assert!(ledger.needed("a", 1, secs(160), true));
        } else {
            ledger.forget_loaded(None);
        }
        assert_eq!(ledger.step("a", 1), super::ledger::NextLoad::NeedPlan);
        assert_eq!(ledger.held_bytes(), 0);
    }
}

#[test]
fn inv_strategy_01_a_finished_leading_plan_does_not_imply_a_whole_source() {
    let mut ledger = ledger();
    assert!(ledger.needed("a", 1, secs(100), true));
    ledger.set_plan("a", plan(1));
    ledger.record_chunk("a", 0, vec![("c0".into(), 10)], None, u64::MAX);
    assert!(ledger.needed("a", usize::MAX, secs(101), true), "leading plan suppressed whole");
}

fn plan(count: usize) -> Plan {
    Plan { playlists: Vec::new(), chunks: (0..count).map(|index| Chunk { segment: format!("c{index}"), init: None }).collect(), headers: HashMap::new() }
}

fn load_unit(ledger: &mut PrefetchLedger, key: &str, now: Duration) -> bool {
    let needed = ledger.needed(key, 1, now, true);
    if needed {
        ledger.record_unit(key);
    }
    needed
}

#[test]
fn inv_prefetch_01_loads_first_and_skips_a_loaded_source_inside_the_window() {
    let mut ledger = ledger();
    assert!(load_unit(&mut ledger, "a", secs(100)));
    assert!(!load_unit(&mut ledger, "a", secs(130)));
    assert!(!load_unit(&mut ledger, "a", secs(159)));
}

#[test]
fn inv_prefetch_01_reopens_after_window() {
    let mut ledger = ledger();
    load_unit(&mut ledger, "a", secs(100));
    assert!(load_unit(&mut ledger, "a", secs(160)));
}

#[test]
fn inv_prefetch_01_a_larger_target_is_needed_until_the_chunks_are_loaded() {
    let mut ledger = ledger();
    assert!(ledger.needed("a", 1, secs(100), true));
    ledger.set_plan("a", plan(5));
    ledger.record_chunk("a", 0, vec![("c0".into(), 10)], None, u64::MAX);
    assert!(!ledger.needed("a", 1, secs(110), true));
    assert!(ledger.needed("a", 3, secs(110), false));
    assert_eq!(ledger.loaded_chunks("a"), 1);
}

#[test]
fn inv_prefetch_01_entries_stay_within_max_and_oldest_is_dropped() {
    let mut ledger = ledger();
    for index in 0..501u64 {
        load_unit(&mut ledger, &format!("k{index}"), Duration::from_millis(index * 10));
    }
    let oldest_dropped = load_unit(&mut ledger, "k0", secs(6));
    let newest_kept = load_unit(&mut ledger, "k500", secs(6));
    assert_eq!(ledger.dedupe_count(), 500);
    assert!(oldest_dropped);
    assert!(!newest_kept);
}

#[test]
fn inv_prefetch_07_cancel_marks_queued_requests_until_they_drain() {
    let mut ledger = ledger();
    ledger.enqueue("a");
    ledger.enqueue("a");
    let had_queued = ledger.cancel("a", false);
    let first = ledger.begin("a");
    let second = ledger.begin("a");
    ledger.enqueue("a");
    let after_drain = ledger.begin("a");
    assert!(had_queued);
    assert!(first);
    assert!(second);
    assert!(!after_drain);
}

#[test]
fn inv_prefetch_07_begin_without_cancel_is_not_cancelled() {
    let mut ledger = ledger();
    ledger.enqueue("b");
    assert!(!ledger.begin("b"));
}

#[test]
fn inv_prefetch_07_cancel_counts_only_when_something_was_pending() {
    let mut ledger = ledger();
    assert!(!ledger.cancel("a", false));
    assert_eq!(ledger.counters.cancelled, 0);

    assert!(!ledger.cancel("a", true));
    assert_eq!(ledger.counters.cancelled, 1);

    ledger.enqueue("b");
    assert!(ledger.cancel("b", false));
    assert_eq!(ledger.counters.cancelled, 2);
}

#[test]
fn inv_prefetch_01_set_limits_changes_the_repeat_skip_window_and_the_remembered_count() {
    let mut ledger = PrefetchLedger::new(Duration::from_secs(60), 100);
    assert!(load_unit(&mut ledger, "a", secs(0)));
    assert!(!load_unit(&mut ledger, "a", secs(20)));

    ledger.set_limits(Duration::from_secs(10), 1);

    assert!(load_unit(&mut ledger, "a", secs(20)));
    assert!(load_unit(&mut ledger, "b", secs(21)));
    assert_eq!(ledger.dedupe_count(), 1);
}

#[test]
fn inv_prefetch_01_inv_prefetch_07_random_operations_keep_the_dedupe_bounded_and_the_cancel_flags_exact() {
    use crate::seeded::Dice;
    let mut dice = Dice::new("inv_prefetch_01_inv_prefetch_07_random_operations_keep_the_dedupe_bounded_and_the_cancel_flags_exact");
    let window = Duration::from_secs(10);
    let max = 8;
    let mut ledger = PrefetchLedger::new(window, max);
    let mut queued: HashMap<String, u32> = HashMap::new();
    let mut flagged: HashMap<String, bool> = HashMap::new();
    let mut admitted_at: HashMap<String, (Duration, Duration)> = HashMap::new();
    let mut now = Duration::ZERO;
    let (mut admits, mut covered) = (0, 0);
    for step in 0..6_000 {
        now += Duration::from_millis(dice.below(900));
        let key = format!("u{}", dice.below(14));
        match dice.below(6) {
            0..=2 => {
                let before = admitted_at.get(&key).copied();
                let verdict = load_unit(&mut ledger, &key, now);
                if verdict {
                    admits += 1;
                    admitted_at.insert(key.clone(), (now, Duration::ZERO));
                } else {
                    covered += 1;
                    let (at, _) = before.unwrap_or_else(|| panic!("step {step}: {key} was refused without ever being loaded"));
                    assert!(now - at < window, "step {step}: {key} refused although the window ended");
                }
                let again = load_unit(&mut ledger, &key, now);
                assert!(!again, "step {step}: loading {key} twice at the same instant gave a second slot");
                assert!(ledger.dedupe_count() <= max, "step {step}: {} dedupe entries over the limit {max}", ledger.dedupe_count());
            }
            3 => {
                ledger.enqueue(&key);
                *queued.entry(key).or_insert(0) += 1;
            }
            4 => {
                let waiting = queued.get(&key).copied().unwrap_or(0);
                if waiting == 0 {
                    continue;
                }
                let expected = flagged.get(&key).copied().unwrap_or(false);
                assert_eq!(ledger.begin(&key), expected, "step {step}: begin of {key} with {waiting} queued");
                queued.insert(key.clone(), waiting - 1);
                if waiting == 1 {
                    flagged.remove(&key);
                }
            }
            _ => {
                let had_queued = queued.get(&key).copied().unwrap_or(0) > 0;
                assert_eq!(ledger.cancel(&key, false), had_queued, "step {step}: cancel of {key}");
                if had_queued {
                    flagged.insert(key.clone(), true);
                }
            }
        }
    }
    assert!(admits > 200 && covered > 200, "the walk is one-sided: {admits} admitted, {covered} covered");
}
