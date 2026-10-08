use super::*;

#[test]
fn no_target_below_budget() {
    let limits = Limits::default();
    assert_eq!(eviction_target(&limits, limits.budget - 1), None);
    assert_eq!(eviction_target(&limits, 0), None);
}

#[test]
fn target_is_shrink_to_times_budget_at_budget() {
    let limits = Limits { budget: 1000, min_budget: 100, ..Limits::default() };
    assert_eq!(eviction_target(&limits, 1000), Some(900));
}

#[test]
fn target_is_shrink_to_times_budget_above_budget() {
    let limits = Limits { budget: 1000, min_budget: 100, shrink_to: 0.5, ..Limits::default() };
    assert_eq!(eviction_target(&limits, 5000), Some(500));
}

#[test]
fn shrink_to_one_targets_budget() {
    let limits = Limits { budget: 1000, min_budget: 100, shrink_to: 1.0, ..Limits::default() };
    assert_eq!(eviction_target(&limits, 1200), Some(1000));
}
