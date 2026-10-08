mod gate;
mod ledger;
mod load_source;
mod loader;
mod runner;
mod settled;
mod source;
mod window;

pub use ledger::{Counters, PrefetchLedger};
pub use loader::{LoadError, Loader, ProxyLoader};
pub use runner::Prefetcher;
pub use settled::{Outcome, Settled};
pub use source::{Source, SourceKind};
pub use window::{diff_window, wanted_jobs, wanted_page_jobs, PageWindow, PrefetchJob, PrefetchWindow, WindowDiff};

#[cfg(test)]
mod mock_loader;
#[cfg(test)]
mod ledger_tests;
#[cfg(test)]
mod proxy_loader_tests;
#[cfg(test)]
mod runner_tests;
#[cfg(test)]
mod window_tests;
