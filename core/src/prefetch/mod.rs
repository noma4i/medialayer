mod ledger;
mod window;

pub use ledger::{Counters, PrefetchLedger};
pub use window::{diff_window, wanted_jobs, PrefetchJob, PrefetchWindow, WindowDiff};

#[cfg(test)]
mod ledger_tests;
#[cfg(test)]
mod window_tests;
