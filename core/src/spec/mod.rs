mod types;
mod validate;

pub use types::{AudioSpec, CacheSpec, Evict, ImageSpec, MediaSpec, PlaybackSpec, PoolSpec, PrefetchSpec, VideoSpec};
pub use validate::{type_name, SpecError};

#[cfg(test)]
mod spec_tests;
