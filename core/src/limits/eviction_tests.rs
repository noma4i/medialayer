use super::*;

#[test]
fn no_target_below_budget() {
    let limits = Limits::default();
    assert_eq!(eviction_target(&limits, limits.budget - 1), None);
    assert_eq!(eviction_target(&limits, 0), None);
}

#[test]
fn target_is_shrink_to_times_budget_from_the_budget_up() {
    let cases = [(0.9, 1000, 900), (0.5, 5000, 500), (1.0, 1200, 1000)];
    for (shrink_to, used, target) in cases {
        let limits = Limits { budget: 1000, min_budget: 100, shrink_to, ..Limits::default() };
        assert_eq!(eviction_target(&limits, used), Some(target), "shrink_to {shrink_to} used {used}");
    }
}
