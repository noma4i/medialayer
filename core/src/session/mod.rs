mod election;
mod recovery;
mod route_staleness;
mod suspensions;
mod watchdog_policy;

pub use election::{Context, LeaseOrder, Election, Intent, Order, Policy, ResumeMode};
pub use recovery::{next_action, Action, Event, RecoveryState, FOREGROUND_STEPS};
pub use route_staleness::RouteStaleness;
pub use suspensions::{check_groups, SuspensionError, Suspensions};
pub use watchdog_policy::{
    evaluate, has_clock_advanced, window_delay_ms, Decision, Progress, Sample, Verdict, WatchdogConfig,
};

#[cfg(test)]
mod election_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod route_staleness_tests;
#[cfg(test)]
mod suspensions_tests;
#[cfg(test)]
mod watchdog_policy_tests;
