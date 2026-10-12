use super::gate::{Gate, LoadPermit};
use super::ledger::{Counters, Evicted, NextLoad, PrefetchLedger};
use super::load_source::{bounded, is_hls, load_unit, read_plan};
use crate::hls::Chunk;
use super::loader::Loader;
use crate::proxy::Priority;
use super::settled::{Outcome, Settled};
use super::source::Source;
use super::window::{diff_window, wanted_jobs, wanted_page_jobs, PageWindow, PrefetchJob, PrefetchWindow};
use crate::cache::Clock;
use crate::limits::{LeadingChunks, Limits, MIB};
use crate::pages::Sliced;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch, Notify};

const EVENT_CAPACITY: usize = 256;

struct Job {
    id: u64,
    cancel: Arc<Notify>,
    active: Arc<AtomicBool>,
    goal: watch::Sender<usize>,
    distance: Arc<AtomicUsize>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum WindowKey {
    Stage,
    Image(String),
}

struct State {
    ledger: PrefetchLedger,
    windows: HashMap<WindowKey, Vec<PrefetchJob>>,
    last: HashMap<WindowKey, (Vec<PrefetchJob>, Vec<Source>)>,
    running: HashMap<String, Job>,
    next_id: u64,
    busy: bool,
}

impl State {
    fn cancel_job(&mut self, key: &str) {
        let Some(job) = self.running.remove(key) else { return };
        self.ledger.cancel(key, job.active.load(Ordering::SeqCst));
        job.cancel.notify_one();
    }

    fn finish_job(&mut self, key: &str, id: u64) {
        if self.running.get(key).is_some_and(|job| job.id == id) {
            self.running.remove(key);
        }
    }

    fn is_wanted(&self, key: &str) -> bool {
        self.windows.values().any(|jobs| jobs.iter().any(|job| job.key == key))
    }
}

#[derive(Clone)]
struct Tuning {
    window: PrefetchWindow,
    leading: LeadingChunks,
    budget: u64,
    timeout: Duration,
}

impl Tuning {
    fn from_limits(limits: &Limits) -> Tuning {
        Tuning {
            window: PrefetchWindow { from: limits.prefetch_from as usize, to: limits.prefetch_to as usize },
            leading: limits.leading_chunks,
            budget: u64::from(limits.prefetch_budget_mib) * MIB,
            timeout: Duration::from_secs(limits.timeout_seconds.into()),
        }
    }

    fn target(&self, source: &Source, distance: usize) -> usize {
        if is_hls(source) {
            self.leading.for_distance(distance)
        } else {
            1
        }
    }
}

struct Shared {
    loader: Arc<dyn Loader>,
    tuning: Mutex<Tuning>,
    image_windows: Mutex<HashMap<String, PageWindow>>,
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
            image_windows: Mutex::new(HashMap::new()),
            gate: Gate::new(limits.max_concurrent as usize),
            clock,
            state: Mutex::new(State { ledger, windows: HashMap::new(), last: HashMap::new(), running: HashMap::new(), next_id: 0, busy: false }),
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

    pub fn window_range(&self, active: usize, count: usize) -> Range<usize> {
        self.shared.tuning().window.range(active, count)
    }

    pub fn image_window_range(&self, name: &str, active: usize, count: usize) -> Range<usize> {
        self.image_window(name).range(active, count)
    }

    pub fn set_window(&self, active: usize, sources: impl Into<Sliced<Source>>) {
        let sources = sources.into();
        let wanted = wanted_jobs(active, &sources, self.shared.tuning().window);
        let played = sources.get(active).map(|source| source.url.clone());
        self.apply_window(WindowKey::Stage, wanted, sources.into_present().collect(), played);
    }

    pub fn set_image_windows(&self, windows: HashMap<String, PageWindow>) {
        *self.shared.image_windows.lock().unwrap_or_else(PoisonError::into_inner) = windows;
    }

    pub fn set_image_window(&self, name: &str, active: usize, pages: impl Into<Sliced<Vec<Source>>>) {
        let pages = pages.into();
        let wanted = wanted_page_jobs(active, &pages, self.image_window(name));
        self.apply_window(WindowKey::Image(name.to_string()), wanted, pages.into_present().flatten().collect(), None);
    }

    fn image_window(&self, name: &str) -> PageWindow {
        self.shared.image_windows.lock().unwrap_or_else(PoisonError::into_inner).get(name).copied().unwrap_or(PageWindow::DEFAULT)
    }

    fn apply_window(&self, key: WindowKey, wanted: Vec<PrefetchJob>, sources: Vec<Source>, played: Option<String>) {
        let tuning = self.shared.tuning();
        let wanted_urls: HashSet<&str> = wanted.iter().map(|job| job.key.as_str()).collect();
        let by_url: HashMap<&str, &Source> =
            sources.iter().filter(|source| wanted_urls.contains(source.url.as_str())).map(|source| (source.url.as_str(), source)).collect();
        let now = Duration::from_millis(self.shared.clock.now_ms());
        let mut state = self.shared.state();
        let previous = state.windows.remove(&key).unwrap_or_default();
        let diff = diff_window(&previous, &wanted);
        if wanted.is_empty() {
            state.last.remove(&key);
        } else {
            state.last.insert(key.clone(), (wanted.clone(), by_url.values().map(|source| (*source).clone()).collect()));
            state.windows.insert(key, wanted.clone());
        }
        for job in &diff.to_cancel {
            if !state.is_wanted(&job.key) {
                state.cancel_job(&job.key);
                state.ledger.set_distance(&job.key, usize::MAX);
            }
        }
        if let Some(played) = played {
            state.ledger.mark_played(&played);
        }
        let entering: HashSet<&str> = diff.to_start.iter().map(|job| job.key.as_str()).collect();
        for job in &wanted {
            if let Some(source) = by_url.get(job.key.as_str()) {
                self.start(&mut state, (*source).clone(), job.priority.unsigned_abs() as usize, entering.contains(job.key.as_str()), now, &tuning);
            }
        }
    }

    pub fn cancel(&self, url: &str) {
        let mut state = self.shared.state();
        for jobs in state.windows.values_mut() {
            jobs.retain(|job| job.key != url);
        }
        state.cancel_job(url);
    }

    pub fn set_foreground_busy(&self, busy: bool) {
        {
            let mut state = self.shared.state();
            if state.busy == busy {
                return;
            }
            state.busy = busy;
            if busy {
                let running: Vec<String> = state.running.keys().cloned().collect();
                for url in running {
                    state.cancel_job(&url);
                }
                return;
            }
        }
        self.resume();
    }

    pub fn stop_all(&self) {
        Prefetcher::cancel_everything(&self.shared);
    }

    pub fn resume(&self) {
        let windows: Vec<(WindowKey, Vec<PrefetchJob>, Vec<Source>)> =
            self.shared.state().last.iter().map(|(key, (jobs, sources))| (key.clone(), jobs.clone(), sources.clone())).collect();
        for (key, jobs, sources) in windows {
            self.apply_window(key, jobs, sources, None);
        }
    }

    pub fn forget_windows(&self) {
        Prefetcher::cancel_everything(&self.shared);
        self.shared.state().last.clear();
    }

    fn cancel_everything(shared: &Shared) {
        let mut state = shared.state();
        let running: Vec<String> = state.running.keys().cloned().collect();
        for url in running {
            state.cancel_job(&url);
        }
        state.windows.clear();
    }

    pub fn settled(&self) -> broadcast::Receiver<Settled> {
        self.shared.events.subscribe()
    }

    pub fn counters(&self) -> Counters {
        self.shared.state().ledger.counters
    }

    pub fn held_bytes(&self) -> u64 {
        self.shared.state().ledger.held_bytes()
    }

    pub fn live_tasks(&self) -> usize {
        self.shared.live.load(Ordering::SeqCst)
    }

    fn start(&self, state: &mut State, source: Source, distance: usize, entering: bool, now: Duration, tuning: &Tuning) {
        if state.busy {
            return;
        }
        if entering {
            state.ledger.counters.requested += 1;
        }
        let target = tuning.target(&source, distance);
        if let Some(job) = state.running.get(&source.url) {
            job.distance.store(distance, Ordering::SeqCst);
            job.goal.send_replace(target);
            if entering {
                state.ledger.counters.deduped += 1;
            }
            return;
        }
        let needed = state.ledger.needed(&source.url, target, now, entering);
        state.ledger.set_distance(&source.url, distance);
        if !needed {
            if entering {
                state.ledger.counters.deduped += 1;
            }
            return;
        }
        if !entering {
            state.ledger.counters.requested += 1;
        }
        let id = state.next_id;
        state.next_id += 1;
        let (goal, goal_receiver) = watch::channel(target);
        let job = Job { id, cancel: Arc::new(Notify::new()), active: Arc::new(AtomicBool::new(false)), goal, distance: Arc::new(AtomicUsize::new(distance)) };
        state.ledger.enqueue(&source.url);
        let live = LiveTask::new(&self.shared);
        let run = Run { source: source.clone(), id, cancel: job.cancel.clone(), active: job.active.clone(), distance: job.distance.clone(), goal: goal_receiver, tuning: tuning.clone() };
        self.shared.handle.spawn(run_job(live, run));
        state.running.insert(source.url, job);
    }
}

struct Run {
    source: Source,
    id: u64,
    cancel: Arc<Notify>,
    active: Arc<AtomicBool>,
    distance: Arc<AtomicUsize>,
    goal: watch::Receiver<usize>,
    tuning: Tuning,
}

async fn run_job(live: LiveTask, mut run: Run) {
    let shared = live.0.clone();
    let flagged = shared.state().ledger.begin(&run.source.url);
    let outcome = if flagged {
        Outcome::Cancelled
    } else {
        let cancel = run.cancel.clone();
        tokio::select! {
            biased;
            _ = cancel.notified() => Outcome::Cancelled,
            outcome = load(&shared, &mut run) => outcome,
        }
    };
    settle(&shared, &run.source.url, run.id, &outcome);
    drop(live);
    let _ = shared.events.send(Settled { url: run.source.url, outcome });
}

async fn load(shared: &Arc<Shared>, run: &mut Run) -> Outcome {
    if is_hls(&run.source) {
        return load_chunks(shared, run).await;
    }
    let _permit = shared.gate.acquire(None, &run.distance, 0).await;
    mark_started(shared, &run.active);
    let outcome = load_unit(shared.loader.as_ref(), &run.source, run.tuning.timeout).await;
    if outcome == Outcome::Loaded {
        shared.state().ledger.record_unit(&run.source.url);
    }
    outcome
}

fn mark_started(shared: &Shared, active: &AtomicBool) {
    if !active.swap(true, Ordering::SeqCst) {
        shared.state().ledger.counters.started += 1;
    }
}

async fn load_chunks(shared: &Arc<Shared>, run: &mut Run) -> Outcome {
    let source = &run.source;
    let mut permit: Option<LoadPermit> = None;
    loop {
        let step = {
            let mut state = shared.state();
            let step = state.ledger.step(&source.url, *run.goal.borrow());
            if step == NextLoad::Done {
                state.finish_job(&source.url, run.id);
            }
            step
        };
        match step {
            NextLoad::Done => return Outcome::Loaded,
            NextLoad::NeedPlan => {
                permit = Some(shared.gate.acquire(permit.take(), &run.distance, 0).await);
                mark_started(shared, &run.active);
                match read_plan(shared.loader.as_ref(), source, run.tuning.timeout).await {
                    Ok(plan) => shared.state().ledger.set_plan(&source.url, plan),
                    Err(outcome) => return outcome,
                }
            }
            NextLoad::Chunk { index, chunk, init_loaded } => {
                let room = shared.state().ledger.make_room(&source.url, run.distance.load(Ordering::SeqCst), run.tuning.budget);
                let Some(evicted) = room else { return Outcome::OverBudget };
                evict(shared, evicted).await;
                permit = Some(shared.gate.acquire(permit.take(), &run.distance, index).await);
                mark_started(shared, &run.active);
                let loading = load_chunk(shared.loader.as_ref(), source, &chunk, init_loaded, run.tuning.timeout);
                let loaded = tokio::select! {
                    biased;
                    shrunk = run.goal.wait_for(|goal| *goal <= index) => {
                        if shrunk.is_err() {
                            return Outcome::Cancelled;
                        }
                        shared.state().ledger.counters.cancelled += 1;
                        continue;
                    }
                    loaded = loading => loaded,
                };
                let (parts, init) = match loaded {
                    Ok(loaded) => loaded,
                    Err(outcome) => return outcome,
                };
                let (evicted, itself) = shared.state().ledger.record_chunk(&source.url, index, parts, init, run.tuning.budget);
                evict(shared, evicted).await;
                if itself {
                    return Outcome::OverBudget;
                }
            }
        }
    }
}

async fn load_chunk(
    loader: &dyn Loader,
    source: &Source,
    chunk: &Chunk,
    init_loaded: bool,
    timeout: Duration,
) -> Result<(Vec<(String, u64)>, Option<String>), Outcome> {
    let mut parts = Vec::new();
    let mut init = None;
    if let Some(url) = chunk.init.as_ref().filter(|_| !init_loaded) {
        parts.push((url.clone(), bounded(timeout, loader.load(url, &source.headers, Priority::Prefetch)).await?));
        init = Some(url.clone());
    }
    parts.push((chunk.segment.clone(), bounded(timeout, loader.load(&chunk.segment, &source.headers, Priority::Prefetch)).await?));
    Ok((parts, init))
}

async fn evict(shared: &Shared, evicted: Vec<Evicted>) {
    for entry in evicted {
        for url in entry.urls {
            shared.loader.evict(&url, &entry.headers).await;
        }
    }
}

fn settle(shared: &Shared, url: &str, id: u64, outcome: &Outcome) {
    let mut state = shared.state();
    match outcome {
        Outcome::Loaded => state.ledger.counters.completed += 1,
        Outcome::Failed(_) | Outcome::TimedOut => {
            state.ledger.counters.failed += 1;
            for jobs in state.windows.values_mut() {
                jobs.retain(|job| job.key != url);
            }
        }
        Outcome::Cancelled | Outcome::OverBudget => {}
    }
    state.finish_job(url, id);
}
