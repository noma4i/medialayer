use super::*;
use crate::limits::{EvictionStrategy, Limits as CoreLimits};

#[test]
fn every_limit_survives_the_trip_across_the_bridge_types() {
    let core = CoreLimits {
        budget: 2_000_000_000,
        min_budget: 70_000_000,
        shrink_to: 0.75,
        memory_budget: 80_000_000,
        slots: 4,
        prefetch_from: 1,
        prefetch_to: 6,
        depth_seconds: 3,
        timeout_seconds: 45,
        max_concurrent: 5,
        dedupe_window_seconds: 90,
        dedupe_max_entries: 800,
        strategy: EvictionStrategy::ExpireAfter(3600),
    };

    assert_eq!(CoreLimits::from(Limits::from(core)), core);
}

#[test]
fn pool_settings_cross_the_bridge_and_keep_the_profile_tuning_defaults() {
    let settings = PoolSettings { reach: 2, hold_off_ms: 500, trim_delay_ms: 9_000 };

    let core = crate::api::PoolSettings::from(settings);

    assert_eq!((core.reach, core.hold_off_ms, core.trim_delay_ms), (2, 500, 9_000));
    assert_eq!(core.profile, crate::api::PoolSettings::default().profile);
    assert_eq!(default_pool_settings(), PoolSettings { reach: 1, hold_off_ms: 1_000, trim_delay_ms: 30_000 });
}
