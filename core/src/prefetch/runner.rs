use super::gate::Gate;
use super::ledger::{Counters, PrefetchLedger};
use super::load_source::load_source;
use super::loader::Loader;
use super::settled::{Outcome, Settled};
use super::source::Source;
use super::window::{diff_window, wanted_jobs, PrefetchJob, PrefetchWindow};
use crate::cache::Clock;
use crate::limits::Limits;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, Notify};

const EVENT_CAPACITY: usize = 256;

struct Job {
    id: u64,
    cancel: Arc<Notify>,
    active: Arc<AtomicBool>,
}

struct State {
    ledger: PrefetchLedger,
    window_jobs: Vec<PrefetchJob>,
    running: HashMap<String, Job>,
    next_id: u64,
}

impl State {
    fn cancel_job(&mut self, key: &str) {
        let Some(job) = self.running.remove(key) else { return };
        self.ledger.cancel(key, job.active.load(Ordering::SeqCst));
        job.cancel.notify_one();
    }
}

#[derive(Clone)]
struct Tuning {
    window: PrefetchWindow,
    depth: Duration,
    timeout: Duration,
}

impl Tuning {
    fn from_limits(limits: &Limits) -> Tuning {
        Tuning {
            window: PrefetchWindow { from: limits.prefetch_from as usize, to: limits.prefetch_to as usize },
            depth: Duration::from_secs(limits.depth_seconds.into()),
            timeout: Duration::from_secs(limits.timeout_seconds.into()),
        }
    }
}

struct Shared {
    loader: Arc<dyn Loader>,
    tuning: Mutex<Tuning>,
    gate: Arc<Gate>,
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    live: AtomicUsize,
    events: broadcast::Sender<Settled>,
    handle: Handle,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn tuning(&self) -> Tuning {
        self.tuning.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

struct LiveTask(Arc<Shared>);

impl LiveTask {
    fn new(shared: &Arc<Shared>) -> LiveTask {
        shared.live.fetch_add(1, Ordering::SeqCst);
        LiveTask(shared.clone())
    }
}

impl Drop for LiveTask {
    fn drop(&mut self) {
        self.0.live.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Owner(Arc<Shared>);

impl Drop for Owner {
    fn drop(&mut self) {
        Prefetcher::cancel_everything(&self.0);
    }
}

#[derive(Clone)]
pub struct Prefetcher {
    shared: Arc<Shared>,
    _owner: Arc<Owner>,
}

impl Prefetcher {
    pub fn new(loader: Arc<dyn Loader>, limits: &Limits, ledger: PrefetchLedger, clock: Arc<dyn Clock>) -> Prefetcher {
        let shared = Shared {
            loader,
            tuning: Mutex::new(Tuning::from_limits(limits)),
            gate: Gate::new(limits.max_concurrent as usize),
            clock,
            state: Mutex::new(State { ledger, window_jobs: Vec::new(), running: HashMap::new(), next_id: 0 }),
            live: AtomicUsize::new(0),
            events: broadcast::channel(EVENT_CAPACITY).0,
            handle: Handle::current(),
        };
        let shared = Arc::new(shared);
        Prefetcher { _owner: Arc::new(Owner(shared.clone())), shared }
    }

    pub fn set_limits(&self, limits: &Limits) {
        *self.shared.tuning.lock().unwrap_or_else(PoisonError::into_inner) = Tuning::from_limits(limits);
        self.shared.gate.set_limit(limits.max_concurrent as usize);
        let window = Duration::from_secs(limits.dedupe_window_seconds.into());
        self.shared.state().ledger.set_limits(window, limits.dedupe_max_entries as usize);
    }

    pub fn set_window(&self, active: usize, sources: Vec<Option<Source>>) {
        let urls: Vec<Option<String>> = sources.iter().map(|source| source.as_ref().map(|source| source.url.clone())).collect();
        let tuning = self.shared.tuning();
        let wanted = wanted_jobs(active, &urls, tuning.window);
        let wanted_urls: HashSet<&str> = wanted.iter().map(|job| job.key.as_str()).collect();
        let by_url: HashMap<&str, &Source> =
            sources.iter().flatten().filter(|source| wanted_urls.contains(source.url.as_str())).map(|source| (source.url.as_str(), source)).collect();
        let now = Duration::from_millis(self.shared.clock.now_ms());
        let mut state = self.shared.state();
        let diff = diff_window(&state.window_jobs, &wanted);
        for job in &diff.to_cancel {
            state.cancel_job(&job.key);
        }
        for job in &diff.to_start {
            if let Some(source) = by_url.get(job.key.as_str()) {
                self.start(&mut state, (*source).clone(), now, &tuning);
            }
        }
        state.window_jobs = wanted;
    }

    pub fn cancel(&self, url: &str) {
        let mut state = self.shared.state();
        state.window_jobs.retain(|job| job.key != url);
        state.cancel_job(url);
    }

    pub fn stop_all(&self) {
        Prefetcher::cancel_everything(&self.shared);
    }

    fn cancel_everything(shared: &Shared) {
        let mut state = shared.state();
        let running: Vec<String> = state.running.keys().cloned().collect();
        for url in running {
            state.cancel_job(&url);
        }
        state.window_jobs.clear();
    }

    pub fn settled(&self) -> broadcast::Receiver<Settled> {
        self.shared.events.subscribe()
    }

    pub fn counters(&self) -> Counters {
        self.shared.state().ledger.counters
    }

    pub fn live_tasks(&self) -> usize {
        self.shared.live.load(Ordering::SeqCst)
    }

    fn start(&self, state: &mut State, source: Source, now: Duration, tuning: &Tuning) {
        state.ledger.counters.requested += 1;
        if state.running.contains_key(&source.url) || !state.ledger.admit(&source.url, tuning.depth, now) {
            state.ledger.counters.deduped += 1;
            return;
        }
        let id = state.next_id;
        state.next_id += 1;
        let job = Job { id, cancel: Arc::new(Notify::new()), active: Arc::new(AtomicBool::new(false)) };
        state.ledger.enqueue(&source.url);
        let live = LiveTask::new(&self.shared);
        self.shared.handle.spawn(run_job(live, source.clone(), job.id, job.cancel.clone(), job.active.clone(), tuning.clone()));
        state.running.insert(source.url, job);
    }
}

async fn run_job(live: LiveTask, source: Source, id: u64, cancel: Arc<Notify>, active: Arc<AtomicBool>, tuning: Tuning) {
    let shared = live.0.clone();
    let permit = tokio::select! {
        biased;
        _ = cancel.notified() => None,
        permit = shared.gate.acquire() => Some(permit),
    };
    shared.state().ledger.begin(&source.url);
    let outcome = match permit {
        None => Outcome::Cancelled,
        Some(_permit) => {
            active.store(true, Ordering::SeqCst);
            shared.state().ledger.counters.started += 1;
            tokio::select! {
                biased;
                _ = cancel.notified() => Outcome::Cancelled,
                outcome = load_source(shared.loader.as_ref(), &source, tuning.depth, tuning.timeout) => outcome,
            }
        }
    };
    settle(&shared, &source.url, id, &outcome);
    drop(live);
    let _ = shared.events.send(Settled { url: source.url, outcome });
}

fn settle(shared: &Shared, url: &str, id: u64, outcome: &Outcome) {
    let mut state = shared.state();
    match outcome {
        Outcome::Loaded => state.ledger.counters.completed += 1,
        Outcome::Failed(_) | Outcome::TimedOut => {
            state.ledger.counters.failed += 1;
            state.ledger.forget(url);
        }
        Outcome::Cancelled => {}
    }
    if state.running.get(url).is_some_and(|job| job.id == id) {
        state.running.remove(url);
    }
}
