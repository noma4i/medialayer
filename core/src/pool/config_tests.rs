use super::test_support::*;
use super::*;
use crate::limits::Limits;

fn tuning() -> ProfileTuning {
    ProfileTuning { low_ram_below_mb: 3072, low_slots: 2, low_reach: 0 }
}

#[test]
fn inv_pool_14_profile_picks_the_base_or_the_low_ram_config() {
    let custom = ProfileTuning { low_ram_below_mb: 100, low_slots: 1, low_reach: 1 };
    let base = PoolConfig::new(3, 1);
    let shape = |config: PoolConfig| (config.slots, config.reach);
    let cases = [
        ("normal keeps the base", PoolConfig::new(3, 1), DeviceProfile { ram_mb: 8192, low_ram: false }, tuning(), shape(base.clone())),
        ("low ram flag", PoolConfig::new(3, 1), DeviceProfile { ram_mb: 8192, low_ram: true }, tuning(), (2, 0)),
        ("small ram without the flag", PoolConfig::new(3, 1), DeviceProfile { ram_mb: 2048, low_ram: false }, tuning(), (2, 0)),
        ("custom tuning, slots never below two", PoolConfig::new(4, 1), DeviceProfile { ram_mb: 8192, low_ram: true }, custom, (2, 1)),
    ];
    assert_eq!(base.clone().for_profile(DeviceProfile { ram_mb: 8192, low_ram: false }, tuning()), base);
    for (name, config, profile, tuning, expected) in cases {
        assert_eq!(shape(config.for_profile(profile, tuning)), expected, "{name}");
    }
}

#[test]
fn inv_pool_14_low_and_normal_profiles_give_different_slot_plans() {
    let normal = PoolConfig::new(3, 1);
    let low = normal.clone().for_profile(DeviceProfile { ram_mb: 1024, low_ram: true }, tuning());
    let loads = |config: PoolConfig| {
        Ring::new(config).plan(0, 5, Some(5), &keys(20)).iter().filter(|command| matches!(command, Command::Load { .. })).count()
    };
    assert_eq!((loads(normal), loads(low)), (3, 1));
}

#[test]
fn inv_pool_12_start_muted_slots_load_without_mute_commands_and_unmute_on_request() {
    let mut pool = ring_with(3, 1, |config| config.start_muted = true);
    let commands = pool.plan(0, 5, Some(5), &keys(20));
    assert!(commands.iter().all(|command| !matches!(command, Command::Mute { .. } | Command::Unmute { .. })));
    assert_eq!(pool.set_muted_all(false).len(), 3);
}
