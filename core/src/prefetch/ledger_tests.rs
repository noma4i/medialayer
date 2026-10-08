use super::*;
use std::time::Duration;

fn secs(value: u64) -> Duration {
    Duration::from_secs(value)
}

fn ledger() -> PrefetchLedger {
    PrefetchLedger::new(secs(60), 500)
}

#[test]
fn admits_first_and_dedupes_shallower_inside_window() {
    let mut ledger = ledger();
    assert!(ledger.admit("a", secs(2), secs(100)));
    assert!(!ledger.admit("a", secs(2), secs(130)));
    assert!(!ledger.admit("a", secs(1), secs(159)));
}

#[test]
fn reopens_after_window() {
    let mut ledger = ledger();
    ledger.admit("a", secs(2), secs(100));
    assert!(ledger.admit("a", secs(2), secs(160)));
}

#[test]
fn deeper_request_extends_and_raises_the_covered_depth() {
    let mut ledger = ledger();
    ledger.admit("a", secs(2), secs(100));
    assert!(ledger.admit("a", secs(5), secs(110)));
    assert!(!ledger.admit("a", secs(4), secs(120)));
}

#[test]
fn forget_reopens_key() {
    let mut ledger = ledger();
    ledger.admit("a", secs(2), secs(100));
    ledger.forget("a");
    assert!(ledger.admit("a", secs(2), secs(101)));
}

#[test]
fn entries_stay_within_max_and_oldest_is_dropped() {
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
fn limits_are_parameters() {
    let mut ledger = PrefetchLedger::new(secs(10), 2);
    ledger.admit("a", secs(1), secs(0));
    ledger.admit("b", secs(1), secs(1));
    ledger.admit("c", secs(1), secs(2));
    assert_eq!(ledger.dedupe_count(), 2);
    assert!(ledger.admit("a", secs(1), secs(3)));
    assert!(ledger.admit("b", secs(1), secs(11)));
}

#[test]
fn cancel_marks_queued_requests_until_they_drain() {
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
fn begin_without_cancel_is_not_cancelled() {
    let mut ledger = ledger();
    ledger.enqueue("b");
    assert!(!ledger.begin("b"));
}

#[test]
fn cancel_counts_only_when_something_was_pending() {
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
fn set_limits_changes_the_repeat_skip_window_and_the_remembered_count() {
    let mut ledger = PrefetchLedger::new(Duration::from_secs(60), 100);
    let depth = Duration::from_secs(2);
    assert!(ledger.admit("a", depth, Duration::from_secs(0)));
    assert!(!ledger.admit("a", depth, Duration::from_secs(20)));

    ledger.set_limits(Duration::from_secs(10), 1);

    assert!(ledger.admit("a", depth, Duration::from_secs(20)));
    assert!(ledger.admit("b", depth, Duration::from_secs(21)));
    assert_eq!(ledger.dedupe_count(), 1);
}
