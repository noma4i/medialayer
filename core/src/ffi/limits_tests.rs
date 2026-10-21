use super::*;
use crate::limits::{EvictionStrategy, LeadingChunks, Limits as CoreLimits};

#[test]
fn inv_limits_06_every_limit_survives_the_trip_across_the_bridge_types() {
    let core = CoreLimits {
        free_space_fraction: 0.25,
        budget: 2_000_000_000,
        min_budget: 70_000_000,
        shrink_to: 0.75,
        memory_budget: 80_000_000,
        slots: 4,
        prefetch_from: 1,
        prefetch_to: 6,
        leading_chunks: LeadingChunks::new(&[2, 2, 1]).unwrap(),
        prefetch_budget_mib: 96,
        timeout_seconds: 45,
        max_concurrent: 5,
        dedupe_window_seconds: 90,
        dedupe_max_entries: 800,
        playback_chunks_ahead: 5,
        strategy: EvictionStrategy::ExpireAfter(3600),
    };

    assert_eq!(CoreLimits::try_from(Limits::from(core)), Ok(core));
}

#[test]
fn inv_limits_06_pool_settings_cross_the_bridge_with_their_low_memory_options() {
    let settings = PoolSettings { reach: 2, hold_off_ms: 500, trim_delay_ms: 9_000, low_ram_slots: 4, low_ram_reach: 1 };

    let core = crate::api::PoolSettings::from(settings);

    assert_eq!((core.reach, core.hold_off_ms, core.trim_delay_ms), (2, 500, 9_000));
    assert_eq!((core.profile.low_slots, core.profile.low_reach), (4, 1));
    let defaults = default_pool_settings();
    assert_eq!(defaults, PoolSettings { reach: 1, hold_off_ms: 1_000, trim_delay_ms: 30_000, low_ram_slots: 2, low_ram_reach: 0 });
    assert_eq!(crate::api::PoolSettings::from(defaults), crate::api::PoolSettings::default());
}

#[test]
fn inv_limits_06_an_image_window_crosses_the_bridge_with_its_name_and_its_three_bounds() {
    let window = ImageWindow { name: "avatar".to_string(), behind: 2, from: 1, to: 3 };

    let (name, core) = <(String, crate::prefetch::PageWindow)>::from(&window);

    assert_eq!((name.as_str(), core), ("avatar", crate::prefetch::PageWindow { behind: 2, from: 1, to: 3 }));
}
