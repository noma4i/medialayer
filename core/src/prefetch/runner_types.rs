use super::runner::Prefetcher;
use super::gate::Gate;
use super::ledger::PrefetchLedger;
use super::load_source::is_hls;
use super::loader::Loader;
use super::settled::Settled;
use super::source::Source;
use super::window::{PageWindow, PrefetchJob, PrefetchWindow};
use crate::cache::Clock;
use crate::limits::{LeadingChunks, Limits, MIB};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch, Notify};

pub(super) const EVENT_CAPACITY: usize = 256;
pub(super) const WHOLE_MEDIA_PARALLEL: usize = 1;
pub(super) const WHOLE_IMAGE_PARALLEL: usize = 2;
pub(super) const WINDOW_OWNER: u64 = 0;

pub(super) struct WholeHold {
    pub(super) source: Source,
    pub(super) owners: HashSet<u64>,
}

pub(super) struct Job {
    pub(super) id: u64,
    pub(super) url: String,
    pub(super) cancel: Arc<Notify>,
    pub(super) active: Arc<AtomicBool>,
    pub(super) goal: watch::Sender<usize>,
    pub(super) distance: Arc<AtomicUsize>,
    pub(super) whole: bool,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum WindowKey {
    Stage,
    Image(String),
}

pub(super) struct PrefetchState {
    pub(super) ledger: PrefetchLedger,
    pub(super) windows: HashMap<WindowKey, Vec<PrefetchJob>>,
    pub(super) last: HashMap<WindowKey, (Vec<PrefetchJob>, HashMap<String, Source>)>,
    pub(super) running: HashMap<String, Job>,
    pub(super) wholes: HashMap<String, WholeHold>,
    pub(super) mixed: HashSet<String>,
    pub(super) next_id: u64,
    pub(super) busy: bool,
    pub(super) suspended: bool,
}

impl PrefetchState {
    pub(super) fn owns(&self, key: &str, id: u64) -> bool {
        self.running.get(key).is_some_and(|job| job.id == id)
    }

    pub(super) fn cancel_job(&mut self, key: &str) {
        let Some(job) = self.running.remove(key) else { return };
        self.ledger.cancel(key, job.active.load(Ordering::SeqCst));
        job.cancel.notify_one();
    }

    pub(super) fn finish_job(&mut self, key: &str, id: u64) {
        if self.running.get(key).is_some_and(|job| job.id == id) {
            self.running.remove(key);
        }
    }

    pub(super) fn is_wanted(&self, key: &str) -> bool {
        self.windows.values().any(|jobs| jobs.iter().any(|job| job.key == key))
    }
}

#[derive(Clone)]
pub(super) struct Tuning {
    pub(super) window: PrefetchWindow,
    pub(super) leading: LeadingChunks,
    pub(super) budget: u64,
    pub(super) timeout: Duration,
    pub(super) max_entries: usize,
}

impl Tuning {
    pub(super) fn from_limits(limits: &Limits) -> Tuning {
        Tuning {
            window: PrefetchWindow { from: limits.prefetch_from as usize, to: limits.prefetch_to as usize },
            leading: limits.leading_chunks,
            budget: u64::from(limits.prefetch_budget_mib) * MIB,
            timeout: Duration::from_secs(limits.timeout_seconds.into()),
            max_entries: limits.dedupe_max_entries as usize,
        }
    }

    pub(super) fn target(&self, source: &Source, distance: usize) -> usize {
        if is_hls(source) {
            self.leading.for_distance(distance)
        } else {
            1
        }
    }
}

pub(super) struct Shared {
    pub(super) failures: Mutex<Arc<crate::failures::Failures>>,
    pub(super) loader: Arc<dyn Loader>,
    pub(super) tuning: Mutex<Tuning>,
    pub(super) image_windows: Mutex<HashMap<String, PageWindow>>,
    pub(super) gate: Arc<Gate>,
    pub(super) whole_gates: [Arc<Gate>; 2],
    pub(super) clock: Arc<dyn Clock>,
    pub(super) state: Mutex<PrefetchState>,
    pub(super) live: AtomicUsize,
    pub(super) events: broadcast::Sender<Settled>,
    #[cfg(test)]
    pub(super) finished: tokio::sync::watch::Sender<usize>,
    pub(super) handle: Handle,
}

impl Shared {
    pub(super) fn state(&self) -> MutexGuard<'_, PrefetchState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn tuning(&self) -> Tuning {
        self.tuning.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

pub(super) struct LiveTask(pub(super) Arc<Shared>);

impl LiveTask {
    pub(super) fn new(shared: &Arc<Shared>) -> LiveTask {
        shared.live.fetch_add(1, Ordering::SeqCst);
        #[cfg(test)]
        crate::test_signal::fire();
        LiveTask(shared.clone())
    }
}

impl Drop for LiveTask {
    fn drop(&mut self) {
        self.0.live.fetch_sub(1, Ordering::SeqCst);
        #[cfg(test)]
        crate::test_signal::fire();
    }
}

pub(super) struct PrefetchOwner(pub(super) Arc<Shared>);

impl Drop for PrefetchOwner {
    fn drop(&mut self) {
        Prefetcher::cancel_everything(&self.0);
    }
}

pub(super) struct StartJob {
    pub(super) key: String,
    pub(super) source: Source,
    pub(super) distance: usize,
    pub(super) entering: bool,
}

pub(super) struct Run {
    pub(super) key: String,
    pub(super) source: Source,
    pub(super) id: u64,
    pub(super) cancel: Arc<Notify>,
    pub(super) active: Arc<AtomicBool>,
    pub(super) distance: Arc<AtomicUsize>,
    pub(super) goal: watch::Receiver<usize>,
    pub(super) tuning: Tuning,
    pub(super) whole: bool,
}
