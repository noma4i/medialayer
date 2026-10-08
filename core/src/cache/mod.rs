mod clock;
mod entry;
mod error;
mod evict;
mod inner;
mod load;
mod naming;
mod options;
mod persist;
mod policy;
mod reader;
mod save;
mod state;
mod store;
mod writer;

pub use clock::{Clock, SystemClock};
pub use entry::{Kind, KindStats, Stats};
pub use error::CacheError;
pub use options::Options;
pub use reader::Reader;
pub use store::Cache;
pub use writer::Writer;

#[cfg(test)]
mod persist_tests;
#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod eviction_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod test_support;
