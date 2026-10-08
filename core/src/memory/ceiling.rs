use crate::limits::MIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceMemory {
    pub total_mb: u32,
    pub heap_class_mb: Option<u32>,
    pub low_ram: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareTuning {
    pub low_ram_percent: u8,
    pub normal_percent: u8,
    pub heap_percent: u8,
}

impl Default for ShareTuning {
    fn default() -> Self {
        Self { low_ram_percent: 15, normal_percent: 25, heap_percent: 50 }
    }
}

fn share(mb: u32, percent: u8) -> u64 {
    u64::from(mb) * MIB * u64::from(percent) / 100
}

pub fn effective_ceiling(memory_option: u64, device: DeviceMemory, tuning: ShareTuning) -> u64 {
    let percent = if device.low_ram { tuning.low_ram_percent } else { tuning.normal_percent };
    let by_total = share(device.total_mb, percent);
    let by_heap = device.heap_class_mb.map_or(u64::MAX, |heap| share(heap, tuning.heap_percent));
    memory_option.min(by_total).min(by_heap)
}
