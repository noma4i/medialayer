use super::*;
use std::time::Duration;

fn secs(value: u64) -> Duration {
    Duration::from_secs(value)
}

fn ledger() -> PrefetchLedger {
    PrefetchLedger::new(secs(60), 500)
}

#[test]
fn inv_prefetch_01_admits_first_and_dedupes_shallower_inside_window() {
    let mut ledger = ledger();
    assert!(ledger.admit("a", secs(2), secs(100)));
    assert!(!ledger.admit("a", secs(2), secs(130)));
    assert!(!ledger.admit("a", secs(1), secs(159)));
}

#[test]
fn inv_prefetch_01_reopens_after_window() {
    let mut ledger = ledger();
    ledger.admit("a", secs(2), secs(100));
    assert!(ledger.admit("a", secs(2), secs(160)));
}

#[test]
fn inv_prefetch_01_deeper_request_extends_and_raises_the_covered_depth() {
    let mut ledger = ledger();
    ledger.admit("a", secs(2), secs(100));
    assert!(ledger.admit("a", secs(5), secs(110)));
    assert!(!ledger.admit("a", secs(4), secs(120)));
}

#[test]
fn inv_prefetch_01_forget_reopens_key() {
    let mut ledger = ledger();
    ledger.admit("a", secs(2), secs(100));
    ledger.forget("a");
    assert!(ledger.admit("a", secs(2), secs(101)));
}

#[test]
fn inv_prefetch_01_entries_stay_within_max_and_oldest_is_dropped() {
    let mut ledger = ledger();
    for index in 0..501u64 {
        ledger.admit(&format!("k{index}"), secs(2), Duration::from_millis(index * 10));
    }
    let oldest_dropped = ledger.admit("k0", secs(2), secs(6));
    let newest_kept = ledger.admit("k500", secs(2), secs(6));
    assert_eq!(ledger.dedupe_count(), 500);
    assert!(oldest_dropped);
    assert!(!newest_kept);
}

#[test]
fn inv_prefetch_01_limits_are_parameters() {
    let mut ledger = PrefetchLedger::new(secs(10), 2);
    ledger.admit("a", secs(1), secs(0));
    ledger.admit("b", secs(1), secs(1));
    ledger.admit("c", secs(1), secs(2));
    assert_eq!(ledger.dedupe_count(), 2);
    assert!(ledger.admit("a", secs(1), secs(3)));
    assert!(ledger.admit("b", secs(1), secs(11)));
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

    ledger.admit("a", secs(2), secs(100));
    assert!(!ledger.cancel("a", true));
    assert_eq!(ledger.counters.cancelled, 1);
    assert!(ledger.admit("a", secs(2), secs(101)));

    ledger.enqueue("b");
    assert!(ledger.cancel("b", false));
    assert_eq!(ledger.counters.cancelled, 2);
}

#[test]
fn inv_prefetch_01_set_limits_changes_the_repeat_skip_window_and_the_remembered_count() {
    let mut ledger = PrefetchLedger::new(Duration::from_secs(60), 100);
    let depth = Duration::from_secs(2);
    assert!(ledger.admit("a", depth, Duration::from_secs(0)));
    assert!(!ledger.admit("a", depth, Duration::from_secs(20)));

    ledger.set_limits(Duration::from_secs(10), 1);

    assert!(ledger.admit("a", depth, Duration::from_secs(20)));
    assert!(ledger.admit("b", depth, Duration::from_secs(21)));
    assert_eq!(ledger.dedupe_count(), 1);
}

#[test]
fn inv_prefetch_01_inv_prefetch_07_random_operations_keep_the_dedupe_bounded_and_the_cancel_flags_exact() {
    use crate::seeded::Dice;
    use std::collections::HashMap;
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
                let depth = Duration::from_secs(dice.between(1, 4));
                let before = admitted_at.get(&key).copied();
                let verdict = ledger.admit(&key, depth, now);
                if verdict {
                    admits += 1;
                    admitted_at.insert(key.clone(), (now, before.map_or(depth, |(_, deepest)| deepest.max(depth))));
                } else {
                    covered += 1;
                    let (at, deepest) = before.unwrap_or_else(|| panic!("step {step}: {key} was refused without ever being admitted"));
                    assert!(now - at < window && depth <= deepest, "step {step}: {key} refused although the window ended or the request is deeper");
                }
                let again = ledger.admit(&key, depth, now);
                assert!(!again, "step {step}: admitting {key} twice at the same instant gave a second slot");
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
                admitted_at.remove(&key);
            }
        }
    }
    assert!(admits > 200 && covered > 200, "the walk is one-sided: {admits} admitted, {covered} covered");
}
