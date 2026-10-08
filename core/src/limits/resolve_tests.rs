use super::*;

#[test]
fn no_saved_value_gives_defaults() {
    let (limits, error) = resolve(Limits::default(), None);
    assert_eq!(limits, Limits::default());
    assert_eq!(error, None);
}

#[test]
fn saved_value_wins_over_defaults() {
    let (limits, error) = resolve(Limits::default(), Some("budget=2147483648\nslots=5"));
    assert_eq!(limits.budget, 2147483648);
    assert_eq!(limits.slots, 5);
    assert_eq!(limits.timeout_seconds, 30);
    assert_eq!(error, None);
}

#[test]
fn saved_value_wins_over_custom_defaults() {
    let defaults = Limits { slots: 7, ..Limits::default() };
    let (limits, _) = resolve(defaults, Some("budget=2147483648"));
    assert_eq!(limits.slots, 7);
    assert_eq!(limits.budget, 2147483648);
}

#[test]
fn saved_budget_below_min_is_dropped_with_error() {
    let (limits, error) = resolve(Limits::default(), Some("budget=1024"));
    assert_eq!(limits, Limits::default());
    assert_eq!(error, Some(LimitsError::BudgetBelowMin { budget: 1024, min_budget: 64 * 1024 * 1024 }));
}

#[test]
fn unparsable_saved_value_is_dropped_with_error() {
    let (limits, error) = resolve(Limits::default(), Some("budget=lots"));
    assert_eq!(limits, Limits::default());
    assert_eq!(error, Some(LimitsError::InvalidValue { key: "budget" }));
}

#[test]
fn saved_round_trip_of_encoded_limits() {
    let saved = Limits { strategy: EvictionStrategy::ExpireAfter(3600), ..Limits::default() };
    let (limits, error) = resolve(Limits::default(), Some(&saved.encode()));
    assert_eq!(limits, saved);
    assert_eq!(error, None);
}
