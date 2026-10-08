use super::*;

const MIB: u64 = 1024 * 1024;

fn device(total_mb: u32, heap_class_mb: Option<u32>, low_ram: bool) -> DeviceMemory {
    DeviceMemory { total_mb, heap_class_mb, low_ram }
}

#[test]
fn defaults_are_15_percent_for_low_ram_and_25_for_the_rest() {
    let share = ShareTuning::default();
    assert_eq!((share.low_ram_percent, share.normal_percent, share.heap_percent), (15, 25, 50));
}

#[test]
fn a_strong_device_keeps_a_small_setting() {
    assert_eq!(effective_ceiling(64 * MIB, device(8192, None, false), ShareTuning::default()), 64 * MIB);
}

#[test]
fn a_normal_device_caps_a_large_setting_to_a_quarter_of_memory() {
    assert_eq!(effective_ceiling(4096 * MIB, device(4096, None, false), ShareTuning::default()), 1024 * MIB);
}

#[test]
fn a_low_ram_profile_cuts_a_large_setting() {
    let ceiling = effective_ceiling(512 * MIB, device(1024, None, true), ShareTuning::default());
    assert_eq!(ceiling, 1024 * MIB * 15 / 100);
    assert!(ceiling < 512 * MIB);
}

#[test]
fn the_android_heap_class_caps_at_half_of_the_heap() {
    assert_eq!(effective_ceiling(512 * MIB, device(8192, Some(256), false), ShareTuning::default()), 128 * MIB);
}

#[test]
fn a_missing_heap_class_is_ignored() {
    assert_eq!(effective_ceiling(512 * MIB, device(8192, None, false), ShareTuning::default()), 512 * MIB);
}

#[test]
fn the_shares_are_tunable() {
    let share = ShareTuning { low_ram_percent: 10, normal_percent: 50, heap_percent: 25 };
    assert_eq!(effective_ceiling(u64::MAX, device(1000, None, true), share), 1000 * MIB / 10);
    assert_eq!(effective_ceiling(u64::MAX, device(1000, None, false), share), 1000 * MIB / 2);
    assert_eq!(effective_ceiling(u64::MAX, device(8000, Some(100), false), share), 25 * MIB);
}

#[test]
fn a_device_with_unknown_memory_gets_no_room() {
    assert_eq!(effective_ceiling(64 * MIB, device(0, None, false), ShareTuning::default()), 0);
}

#[test]
fn a_zero_setting_stays_zero() {
    assert_eq!(effective_ceiling(0, device(8192, None, false), ShareTuning::default()), 0);
}
