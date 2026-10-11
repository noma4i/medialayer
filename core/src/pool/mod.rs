mod admission;
mod config;
mod lru;
mod pinned;
mod playback;
mod release;
mod retention;
mod ring;
mod stats;

pub use config::{DeviceProfile, PoolConfig, ProfileTuning};
pub use retention::{relieve, Directive, Pressure, Retention, SlotState};
pub use ring::{Command, Holder, Ring};
pub use stats::{ReleaseReason, Released, PoolStats};

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod lru_tests;
#[cfg(test)]
mod memory_tests;
#[cfg(test)]
mod peek_tests;
#[cfg(test)]
mod pinned_tests;
#[cfg(test)]
mod playback_tests;
#[cfg(test)]
mod release_tests;
#[cfg(test)]
mod retention_tests;
#[cfg(test)]
mod ring_tests;
#[cfg(test)]
mod shared_tests;
#[cfg(test)]
mod stats_tests;
#[cfg(test)]
mod walk_tests;
