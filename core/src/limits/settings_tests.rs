use super::*;

#[test]
fn default_matches_v3_values() {
    let limits = Limits::default();
    assert_eq!(limits.budget, 4 * 1024 * 1024 * 1024);
    assert_eq!(limits.min_budget, 64 * 1024 * 1024);
    assert_eq!(limits.shrink_to, 0.9);
    assert_eq!(limits.dedupe_window_seconds, 60);
    assert_eq!(limits.dedupe_max_entries, 500);
    assert_eq!(limits.timeout_seconds, 30);
    assert_eq!(limits.depth_seconds, 2);
    assert_eq!((limits.prefetch_from, limits.prefetch_to), (2, 5));
    assert_eq!(limits.slots, 3);
    assert_eq!(limits.max_concurrent, 2);
    assert_eq!(limits.strategy, EvictionStrategy::Lru);
}

#[test]
fn default_is_valid() {
    assert_eq!(Limits::default().validate(), Ok(()));
}

#[test]
fn budget_below_min_is_rejected() {
    let limits = Limits { budget: 10, min_budget: 20, ..Limits::default() };
    assert_eq!(limits.validate(), Err(LimitsError::BudgetBelowMin { budget: 10, min_budget: 20 }));
}

#[test]
fn budget_equal_to_min_is_accepted() {
    let limits = Limits { budget: 20, min_budget: 20, ..Limits::default() };
    assert_eq!(limits.validate(), Ok(()));
}

#[test]
fn shrink_to_outside_unit_interval_is_rejected() {
    for value in [0.0, -0.5, 1.01, f64::NAN, f64::INFINITY] {
        let limits = Limits { shrink_to: value, ..Limits::default() };
        assert_eq!(limits.validate(), Err(LimitsError::ShrinkToOutOfRange));
    }
}

#[test]
fn shrink_to_one_is_accepted() {
    let limits = Limits { shrink_to: 1.0, ..Limits::default() };
    assert_eq!(limits.validate(), Ok(()));
}

#[test]
fn prefetch_window_from_above_to_is_rejected() {
    let limits = Limits { prefetch_from: 6, prefetch_to: 5, ..Limits::default() };
    assert_eq!(limits.validate(), Err(LimitsError::PrefetchWindowInverted { from: 6, to: 5 }));
}

#[test]
fn prefetch_window_with_equal_ends_is_accepted() {
    let limits = Limits { prefetch_from: 3, prefetch_to: 3, ..Limits::default() };
    assert_eq!(limits.validate(), Ok(()));
}

#[test]
fn zero_in_forbidden_field_is_rejected() {
    let cases = [
        ("budget", Limits { budget: 0, min_budget: 0, ..Limits::default() }),
        ("memory_budget", Limits { memory_budget: 0, ..Limits::default() }),
        ("slots", Limits { slots: 0, ..Limits::default() }),
        ("depth_seconds", Limits { depth_seconds: 0, ..Limits::default() }),
        ("timeout_seconds", Limits { timeout_seconds: 0, ..Limits::default() }),
        ("max_concurrent", Limits { max_concurrent: 0, ..Limits::default() }),
        ("dedupe_window_seconds", Limits { dedupe_window_seconds: 0, ..Limits::default() }),
        ("dedupe_max_entries", Limits { dedupe_max_entries: 0, ..Limits::default() }),
        ("strategy", Limits { strategy: EvictionStrategy::ExpireAfter(0), ..Limits::default() }),
    ];
    for (field, limits) in cases {
        assert_eq!(limits.validate(), Err(LimitsError::Zero { field }), "{field}");
    }
}

#[test]
fn zero_min_budget_and_zero_prefetch_from_are_accepted() {
    let limits = Limits { min_budget: 0, prefetch_from: 0, ..Limits::default() };
    assert_eq!(limits.validate(), Ok(()));
}

#[test]
fn encode_decode_round_trip_for_every_strategy() {
    let strategies = [
        EvictionStrategy::Lru,
        EvictionStrategy::Fifo,
        EvictionStrategy::Lfu,
        EvictionStrategy::LargestFirst,
        EvictionStrategy::ExpireAfter(86400),
    ];
    for strategy in strategies {
        let limits = Limits { strategy, shrink_to: 0.75, slots: 9, ..Limits::default() };
        assert_eq!(Limits::decode(Limits::default(), &limits.encode()), Ok(limits));
    }
}

#[test]
fn encode_is_key_value_lines() {
    let text = Limits::default().encode();
    assert!(text.lines().any(|line| line == "budget=4294967296"));
    assert!(text.lines().any(|line| line == "shrink_to=0.9"));
    assert!(text.lines().any(|line| line == "strategy=lru"));
}

#[test]
fn decode_applies_only_present_keys_over_base() {
    let base = Limits { slots: 7, ..Limits::default() };
    let decoded = Limits::decode(base, "timeout_seconds=10\n").unwrap();
    assert_eq!(decoded.timeout_seconds, 10);
    assert_eq!(decoded.slots, 7);
}

#[test]
fn decode_rejects_unknown_key() {
    assert_eq!(
        Limits::decode(Limits::default(), "color=red"),
        Err(LimitsError::UnknownKey { key: "color".to_string() })
    );
}

#[test]
fn decode_rejects_line_without_equals() {
    assert_eq!(Limits::decode(Limits::default(), "budget=100\nnonsense"), Err(LimitsError::MalformedLine { line: 2 }));
}

#[test]
fn decode_rejects_bad_number_and_bad_strategy() {
    assert_eq!(Limits::decode(Limits::default(), "slots=-1"), Err(LimitsError::InvalidValue { key: "slots" }));
    assert_eq!(Limits::decode(Limits::default(), "strategy=random"), Err(LimitsError::InvalidValue { key: "strategy" }));
    assert_eq!(
        Limits::decode(Limits::default(), "strategy=expire_after:soon"),
        Err(LimitsError::InvalidValue { key: "strategy" })
    );
}

#[test]
fn decode_runs_validation() {
    assert_eq!(Limits::decode(Limits::default(), "slots=0"), Err(LimitsError::Zero { field: "slots" }));
}

#[test]
fn decode_ignores_blank_lines() {
    let decoded = Limits::decode(Limits::default(), "\nslots=4\n\n").unwrap();
    assert_eq!(decoded.slots, 4);
}
