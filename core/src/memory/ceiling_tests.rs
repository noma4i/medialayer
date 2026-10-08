use super::*;

const MIB: u64 = 1024 * 1024;

fn device(total_mb: u32, heap_class_mb: Option<u32>, low_ram: bool) -> DeviceMemory {
    DeviceMemory { total_mb, heap_class_mb, low_ram }
}

#[test]
fn the_ceiling_is_the_setting_capped_by_a_share_of_memory_or_heap() {
    let cases = [
        ("a strong device keeps a small setting", 64 * MIB, device(8192, None, false), 64 * MIB),
        ("a normal device caps to a quarter of memory", 4096 * MIB, device(4096, None, false), 1024 * MIB),
        ("a low ram profile caps to 15 percent", 512 * MIB, device(1024, None, true), 1024 * MIB * 15 / 100),
        ("the android heap class caps to half of the heap", 512 * MIB, device(8192, Some(256), false), 128 * MIB),
        ("a missing heap class is ignored", 512 * MIB, device(8192, None, false), 512 * MIB),
        ("unknown memory gets no room", 64 * MIB, device(0, None, false), 0),
        ("a zero setting stays zero", 0, device(8192, None, false), 0),
    ];
    for (name, setting, memory, expected) in cases {
        assert_eq!(effective_ceiling(setting, memory, ShareTuning::default()), expected, "{name}");
    }
}

#[test]
fn the_shares_are_tunable() {
    let share = ShareTuning { low_ram_percent: 10, normal_percent: 50, heap_percent: 25 };
    assert_eq!(effective_ceiling(u64::MAX, device(1000, None, true), share), 1000 * MIB / 10);
    assert_eq!(effective_ceiling(u64::MAX, device(1000, None, false), share), 1000 * MIB / 2);
    assert_eq!(effective_ceiling(u64::MAX, device(8000, Some(100), false), share), 25 * MIB);
}
