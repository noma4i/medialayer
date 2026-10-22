mod byte_store;
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
mod reservation;
mod save;
mod state;
mod verify;
mod store;
mod writer;

pub use byte_store::{classify_kind, CacheStore, Classify};
pub use clock::{Clock, SystemClock};
#[cfg(test)]
pub use clock::ManualClock;
pub use entry::{Kind, KindStats, CacheStats};
pub use error::CacheError;
pub use options::Options;
pub use reader::Reader;
pub use store::Cache;
pub use writer::Writer;

#[cfg(test)]
mod oracle_tests;
#[cfg(test)]
mod persist_tests;
#[cfg(test)]
mod byte_store_tests;
#[cfg(test)]
mod proxy_cache_tests;
#[cfg(test)]
mod cancel_tests;
#[cfg(test)]
mod concurrency_tests;
#[cfg(test)]
mod ready_reads_tests;
#[cfg(test)]
mod eviction_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod stand_tests;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tone_tests;
#[cfg(test)]
mod optimization_tests;

#[cfg(test)]
mod segment_tests;
