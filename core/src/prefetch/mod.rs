mod gate;
mod ledger;
mod load_source;
mod loader;
mod plan;
mod runner;
mod settled;
mod source;
mod strategy;
mod whole;
mod window;

pub use ledger::{Counters, PrefetchLedger};
pub use loader::{LoadError, Loader, ProxyLoader};
pub use runner::Prefetcher;
pub use settled::{Outcome, Settled};
pub use source::{Source, SourceKind};
pub use strategy::CacheStrategy;
pub use window::{diff_window, wanted_jobs, wanted_page_jobs, PageWindow, PrefetchJob, PrefetchWindow, WindowDiff};

#[cfg(test)]
mod mock_loader;
#[cfg(test)]
mod chunk_tests;
#[cfg(test)]
mod gate_tests;
#[cfg(test)]
mod ledger_tests;
#[cfg(test)]
mod proxy_loader_tests;
#[cfg(test)]
mod runner_tests;
#[cfg(test)]
mod whole_tests;
#[cfg(test)]
mod whole_dedupe_tests;
#[cfg(test)]
mod whole_parallel_tests;
#[cfg(test)]
mod window_tests;
#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod progress_tests;
