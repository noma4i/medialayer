use super::test_support::*;
use super::*;
use crate::limits::Limits;

fn tuning() -> ProfileTuning {
    ProfileTuning { low_ram_below_mb: 3072, low_slots: 2, low_reach: 0, low_buffer_seconds: 1 }
}

#[test]
fn normal_profile_keeps_the_base_config() {
    let base = PoolConfig::new(3, 1);
    assert_eq!(base.clone().for_profile(DeviceProfile { ram_mb: 8192, low_ram: false }, tuning()), base);
}

#[test]
fn low_ram_flag_gives_two_slots_no_reach_and_a_shorter_buffer() {
    let low = PoolConfig::new(3, 1).for_profile(DeviceProfile { ram_mb: 8192, low_ram: true }, tuning());
    assert_eq!((low.slots, low.reach, low.buffer_seconds), (2, 0, 1));
}

#[test]
fn small_ram_counts_as_low_even_without_the_flag() {
    let low = PoolConfig::new(3, 1).for_profile(DeviceProfile { ram_mb: 2048, low_ram: false }, tuning());
    assert_eq!(low.slots, 2);
}

#[test]
fn tuning_values_are_configurable_and_slots_never_drop_below_two() {
    let custom = ProfileTuning { low_ram_below_mb: 100, low_slots: 1, low_reach: 1, low_buffer_seconds: 5 };
    let low = PoolConfig::new(4, 1).for_profile(DeviceProfile { ram_mb: 8192, low_ram: true }, custom);
    assert_eq!((low.slots, low.reach, low.buffer_seconds), (2, 1, 2));
}

#[test]
fn low_and_normal_profiles_give_different_slot_plans() {
    let normal = PoolConfig::new(3, 1);
    let low = normal.clone().for_profile(DeviceProfile { ram_mb: 1024, low_ram: true }, tuning());
    let loads = |config: PoolConfig| {
        Ring::new(config).plan(0, 5, Some(5), &keys(20)).iter().filter(|command| matches!(command, Command::Load { .. })).count()
    };
    assert_eq!((loads(normal), loads(low)), (3, 1));
}

#[test]
fn config_follows_limits_slots_and_depth() {
    let limits = Limits { slots: 4, depth_seconds: 7, ..Limits::default() };
    let config = PoolConfig::from_limits(&limits, 1);
    assert_eq!((config.slots, config.buffer_seconds), (4, 7));
}

#[test]
fn start_muted_slots_load_without_mute_commands_and_unmute_on_request() {
    let mut pool = ring_with(3, 1, |config| config.start_muted = true);
    let commands = pool.plan(0, 5, Some(5), &keys(20));
    assert!(commands.iter().all(|command| !matches!(command, Command::Mute { .. } | Command::Unmute { .. })));
    assert_eq!(pool.set_muted_all(false).len(), 3);
}
