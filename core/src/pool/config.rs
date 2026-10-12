use crate::limits::Limits;

const DEFAULT_SLOT_BYTES: u64 = 24 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceProfile {
    pub ram_mb: u32,
    pub low_ram: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileTuning {
    pub low_ram_below_mb: u32,
    pub low_slots: usize,
    pub low_reach: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolConfig {
    pub slots: usize,
    pub reach: usize,
    pub hold_off_ms: u64,
    pub trim_delay_ms: u64,
    pub start_muted: bool,
    pub slot_bytes_estimate: u64,
}

impl PoolConfig {
    pub fn new(slots: usize, reach: usize) -> Self {
        Self { slots, reach, hold_off_ms: 1_000, trim_delay_ms: 30_000, start_muted: false, slot_bytes_estimate: DEFAULT_SLOT_BYTES }
    }

    pub fn from_limits(limits: &Limits, reach: usize) -> Self {
        Self::new(limits.slots as usize, reach)
    }

    pub fn for_profile(self, profile: DeviceProfile, tuning: ProfileTuning) -> Self {
        if !profile.low_ram && profile.ram_mb >= tuning.low_ram_below_mb {
            return self;
        }
        Self {
            slots: self.slots.min(tuning.low_slots).max(2),
            reach: tuning.low_reach,
            ..self
        }
    }
}
