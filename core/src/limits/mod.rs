mod chunks;
mod eviction;
mod resolve;
mod settings;

pub use chunks::LeadingChunks;
pub use eviction::eviction_target;
pub use resolve::resolve;
pub use settings::{EvictionStrategy, Limits, LimitsError};
pub(crate) use settings::MIB;

#[cfg(test)]
mod eviction_tests;
#[cfg(test)]
mod resolve_tests;
#[cfg(test)]
mod settings_tests;
