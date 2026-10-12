use super::*;

#[test]
fn inv_limits_01_budget_below_min_is_rejected() {
    let limits = Limits { budget: 10, min_budget: 20, ..Limits::default() };
    assert_eq!(limits.validate(), Err(LimitsError::BudgetBelowMin { budget: 10, min_budget: 20 }));
}

#[test]
fn inv_limits_01_shrink_to_outside_unit_interval_is_rejected() {
    for value in [0.0, -0.5, 1.01, f64::NAN, f64::INFINITY] {
        let limits = Limits { shrink_to: value, ..Limits::default() };
        assert_eq!(limits.validate(), Err(LimitsError::ShrinkToOutOfRange));
    }
}

#[test]
fn inv_limits_01_prefetch_window_from_above_to_is_rejected() {
    let limits = Limits { prefetch_from: 6, prefetch_to: 5, ..Limits::default() };
    assert_eq!(limits.validate(), Err(LimitsError::PrefetchWindowInverted { from: 6, to: 5 }));
}

#[test]
fn inv_limits_01_zero_in_forbidden_field_is_rejected() {
    let cases = [
        ("budget", Limits { budget: 0, min_budget: 0, ..Limits::default() }),
        ("memory_budget", Limits { memory_budget: 0, ..Limits::default() }),
        ("slots", Limits { slots: 0, ..Limits::default() }),
        ("prefetch_budget_mib", Limits { prefetch_budget_mib: 0, ..Limits::default() }),
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
fn inv_limits_02_encode_decode_round_trip_for_every_strategy() {
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
fn inv_limits_02_encode_is_key_value_lines() {
    let text = Limits::default().encode();
    assert!(text.lines().any(|line| line == "budget=4294967296"));
    assert!(text.lines().any(|line| line == "shrink_to=0.9"));
    assert!(text.lines().any(|line| line == "strategy=lru"));
}

#[test]
fn inv_limits_02_decode_applies_only_present_keys_over_base() {
    let base = Limits { slots: 7, ..Limits::default() };
    let decoded = Limits::decode(base, "timeout_seconds=10\n").unwrap();
    assert_eq!(decoded.timeout_seconds, 10);
    assert_eq!(decoded.slots, 7);
}

#[test]
fn inv_limits_02_decode_ignores_blank_lines() {
    let decoded = Limits::decode(Limits::default(), "\nslots=4\n\n").unwrap();
    assert_eq!(decoded.slots, 4);
}

#[test]
fn inv_limits_01_boundary_values_are_accepted() {
    let cases = [
        ("budget equal to min", Limits { budget: 20, min_budget: 20, ..Limits::default() }),
        ("shrink_to one", Limits { shrink_to: 1.0, ..Limits::default() }),
        ("equal prefetch ends", Limits { prefetch_from: 3, prefetch_to: 3, ..Limits::default() }),
        ("zero min_budget and prefetch_from", Limits { min_budget: 0, prefetch_from: 0, ..Limits::default() }),
    ];
    for (name, limits) in cases {
        assert_eq!(limits.validate(), Ok(()), "{name}");
    }
}

#[test]
fn inv_limits_02_decode_rejects_bad_text() {
    let cases = [
        ("color=red", LimitsError::UnknownKey { key: "color".to_string() }),
        ("budget=100\nnonsense", LimitsError::MalformedLine { line: 2 }),
        ("slots=-1", LimitsError::InvalidValue { key: "slots" }),
        ("strategy=random", LimitsError::InvalidValue { key: "strategy" }),
        ("strategy=expire_after:soon", LimitsError::InvalidValue { key: "strategy" }),
        ("slots=0", LimitsError::Zero { field: "slots" }),
    ];
    for (text, error) in cases {
        assert_eq!(Limits::decode(Limits::default(), text), Err(error), "{text}");
    }
}
