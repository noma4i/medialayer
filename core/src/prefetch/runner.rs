use super::gate::{Gate, LoadPermit};
use super::ledger::{Counters, Evicted, NextLoad, PrefetchLedger};
use super::load_source::{bounded, is_hls, load_unit, read_plan};
use super::whole::load_whole;
use crate::hls::Chunk;
use super::loader::Loader;
use crate::proxy::Priority;
use super::settled::{Outcome, Settled};
use super::source::{Source, SourceKind};
use super::strategy::CacheStrategy;
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
const WHOLE_MEDIA_PARALLEL: usize = 1;
const WHOLE_IMAGE_PARALLEL: usize = 2;
const WINDOW_OWNER: u64 = 0;

struct Held {
    source: Source,
    owners: HashSet<u64>,
}

struct Job {
    id: u64,
    url: String,
    cancel: Arc<Notify>,
    active: Arc<AtomicBool>,
    goal: watch::Sender<usize>,
    distance: Arc<AtomicUsize>,
    whole: bool,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum WindowKey {
    Stage,
    Image(String),
}

struct State {
    ledger: PrefetchLedger,
    windows: HashMap<WindowKey, Vec<PrefetchJob>>,
    last: HashMap<WindowKey, (Vec<PrefetchJob>, HashMap<String, Source>)>,
    running: HashMap<String, Job>,
    wholes: HashMap<String, Held>,
    next_id: u64,
    busy: bool,
    suspended: bool,
}

impl State {
    fn owns(&self, key: &str, id: u64) -> bool {
        self.running.get(key).is_some_and(|job| job.id == id)
    }

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
    max_entries: usize,
}

impl Tuning {
    fn from_limits(limits: &Limits) -> Tuning {
        Tuning {
            window: PrefetchWindow { from: limits.prefetch_from as usize, to: limits.prefetch_to as usize },
            leading: limits.leading_chunks,
            budget: u64::from(limits.prefetch_budget_mib) * MIB,
            timeout: Duration::from_secs(limits.timeout_seconds.into()),
            max_entries: limits.dedupe_max_entries as usize,
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
    whole_gates: [Arc<Gate>; 2],
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
            whole_gates: [Gate::new(WHOLE_MEDIA_PARALLEL), Gate::new(WHOLE_IMAGE_PARALLEL)],
            clock,
            state: Mutex::new(State { ledger, windows: HashMap::new(), last: HashMap::new(), running: HashMap::new(), wholes: HashMap::new(), next_id: 0, busy: false, suspended: false }),
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
        let keys = sources.map(|source| Some(self.shared.loader.cache_key(&source.url, &source.headers)));
        let wanted = wanted_jobs(active, &keys, self.shared.tuning().window);
        let played = keys.get(active).cloned();
        let sources = keys.into_present().zip(sources.into_present()).collect();
        self.apply_window(WindowKey::Stage, wanted, sources, played);
    }

    pub fn set_image_windows(&self, windows: HashMap<String, PageWindow>) {
        *self.shared.image_windows.lock().unwrap_or_else(PoisonError::into_inner) = windows;
    }

    pub fn set_image_window(&self, name: &str, active: usize, pages: impl Into<Sliced<Vec<Source>>>) {
        let pages = pages.into();
        let keys = pages.map(|page| Some(page.iter().map(|source| self.shared.loader.cache_key(&source.url, &source.headers)).collect::<Vec<_>>()));
        let wanted = wanted_page_jobs(active, &keys, self.image_window(name));
        let sources = keys.into_present().flatten().zip(pages.into_present().flatten()).collect();
        self.apply_window(WindowKey::Image(name.to_string()), wanted, sources, None);
    }

    fn image_window(&self, name: &str) -> PageWindow {
        self.shared.image_windows.lock().unwrap_or_else(PoisonError::into_inner).get(name.split('\u{1f}').next().unwrap_or(name)).copied().unwrap_or(PageWindow::DEFAULT)
    }

    fn apply_window(&self, key: WindowKey, wanted: Vec<PrefetchJob>, mut sources: HashMap<String, Source>, played: Option<String>) {
        let tuning = self.shared.tuning();
        let wanted_keys: HashSet<&str> = wanted.iter().map(|job| job.key.as_str()).collect();
        sources.retain(|key, _| wanted_keys.contains(key.as_str()));
        let now = Duration::from_millis(self.shared.clock.now_ms());
        let mut state = self.shared.state();
        let previous = state.windows.remove(&key).unwrap_or_default();
        let diff = diff_window(&previous, &wanted);
        if wanted.is_empty() {
            state.last.remove(&key);
        } else {
            state.last.insert(key.clone(), (wanted.clone(), sources.clone()));
            state.windows.insert(key, wanted.clone());
        }
        for job in &diff.to_cancel {
            if !state.is_wanted(&job.key) && !state.running.get(&job.key).is_some_and(|job| job.whole) {
                state.cancel_job(&job.key);
                state.ledger.set_distance(&job.key, usize::MAX);
            }
        }
        if let Some(played) = played {
            state.ledger.mark_played(&played);
        }
        let entering: HashSet<&str> = diff.to_start.iter().map(|job| job.key.as_str()).collect();
        for job in &wanted {
            if let Some(source) = sources.get(job.key.as_str()) {
                self.start(&mut state, StartJob { key: job.key.clone(), source: source.clone(), distance: job.priority.unsigned_abs() as usize, entering: entering.contains(job.key.as_str()) }, now, &tuning);
            }
        }
    }

    pub fn cancel(&self, url: &str) {
        let mut state = self.shared.state();
        let mut keys: HashSet<String> = state.running.iter().filter(|(_, job)| job.url == url).map(|(key, _)| key.clone()).collect();
        state.wholes.retain(|key, held| {
            if held.source.url != url { return true; }
            keys.insert(key.clone());
            false
        });
        for (jobs, sources) in state.last.values_mut() {
            sources.retain(|key, source| {
                if source.url != url { return true; }
                keys.insert(key.clone());
                false
            });
            jobs.retain(|job| sources.contains_key(&job.key));
        }
        for jobs in state.windows.values_mut() {
            jobs.retain(|job| !keys.contains(&job.key));
        }
        for key in keys {
            state.cancel_job(&key);
            state.ledger.set_distance(&key, usize::MAX);
        }
    }

    pub fn set_foreground_busy(&self, busy: bool) {
        {
            let mut state = self.shared.state();
            if state.busy == busy {
                return;
            }
            state.busy = busy;
            if busy {
                let running: Vec<String> = state.running.iter().filter(|(_, job)| !job.whole).map(|(url, _)| url.clone()).collect();
                for url in running {
                    state.cancel_job(&url);
                }
                return;
            }
        }
        self.restart();
    }

    pub fn forget_loaded(&self, url: Option<&str>) {
        let mut state = self.shared.state();
        let keys: Vec<String> = state.running.iter().filter(|(_, job)| url.is_none_or(|url| job.url == url)).map(|(key, _)| key.clone()).collect();
        for key in keys {
            state.cancel_job(&key);
        }
        state.ledger.forget_loaded(url);
    }

    pub fn stop_all(&self) {
        Prefetcher::cancel_everything(&self.shared);
    }

    pub fn suspend(&self) {
        self.shared.state().suspended = true;
        Prefetcher::cancel_everything(&self.shared);
    }

    pub fn resume(&self) {
        self.shared.state().suspended = false;
        self.restart();
    }

    fn restart(&self) {
        let windows: Vec<_> =
            self.shared.state().last.iter().map(|(key, (jobs, sources))| (key.clone(), jobs.clone(), sources.clone())).collect();
        for (key, jobs, sources) in windows {
            self.apply_window(key, jobs, sources, None);
        }
        let now = Duration::from_millis(self.shared.clock.now_ms());
        let mut state = self.shared.state();
        let wholes: Vec<_> = state.wholes.iter().map(|(key, held)| (key.clone(), held.source.clone())).collect();
        for (key, source) in wholes {
            self.start_whole(&mut state, key, source, now, false);
        }
    }

    pub fn keep_whole(&self, source: Source) {
        self.hold_whole(WINDOW_OWNER, source);
    }

    pub fn hold_whole(&self, owner: u64, source: Source) {
        if source.strategy != CacheStrategy::Whole {
            return;
        }
        let now = Duration::from_millis(self.shared.clock.now_ms());
        let key = self.shared.loader.cache_key(&source.url, &source.headers);
        let mut state = self.shared.state();
        if !state.wholes.contains_key(&key) && state.wholes.len() >= self.shared.tuning().max_entries {
            state.ledger.counters.requested += 1;
            state.ledger.counters.failed += 1;
            let _ = self.shared.events.send(Settled { url: source.url, outcome: Outcome::Failed("whole queue is full".into()) });
            return;
        }
        state.wholes.entry(key.clone()).or_insert_with(|| Held { source: source.clone(), owners: HashSet::new() }).owners.insert(owner);
        self.start_whole(&mut state, key, source, now, true);
    }

    pub fn release_whole(&self, owner: u64, source: &Source) {
        let key = self.shared.loader.cache_key(&source.url, &source.headers);
        let mut state = self.shared.state();
        Prefetcher::let_go(&mut state, owner, &key);
    }

    pub fn release_owner(&self, owner: u64) {
        let mut state = self.shared.state();
        let keys: Vec<String> = state.wholes.iter().filter(|(_, held)| held.owners.contains(&owner)).map(|(key, _)| key.clone()).collect();
        for key in keys {
            Prefetcher::let_go(&mut state, owner, &key);
        }
    }

    fn let_go(state: &mut State, owner: u64, key: &str) {
        let Some(held) = state.wholes.get_mut(key) else { return };
        held.owners.remove(&owner);
        if !held.owners.is_empty() {
            return;
        }
        state.wholes.remove(key);
        if state.running.get(key).is_some_and(|job| job.whole) && !state.is_wanted(key) {
            state.cancel_job(key);
            state.ledger.set_distance(key, usize::MAX);
        }
    }

    fn start_whole(&self, state: &mut State, key: String, source: Source, now: Duration, entering: bool) {
        if state.suspended {
            return;
        }
        if entering {
            state.ledger.counters.requested += 1;
        }
        if let Some(job) = state.running.get_mut(&key) {
            job.whole = true;
            job.goal.send_replace(usize::MAX);
            if entering {
                state.ledger.counters.deduped += 1;
            }
            return;
        }
        if !state.ledger.needed(&key, usize::MAX, now, entering) {
            state.wholes.remove(&key);
            if entering {
                state.ledger.counters.deduped += 1;
            }
            return;
        }
        if !entering {
            state.ledger.counters.requested += 1;
        }
        self.launch(state, key, source, usize::MAX, true);
    }

    pub fn forget_windows(&self) {
        Prefetcher::cancel_everything(&self.shared);
        let mut state = self.shared.state();
        state.last.clear();
        state.wholes.clear();
    }

    fn cancel_everything(shared: &Shared) {
        let mut state = shared.state();
        let running: Vec<String> = state.running.keys().cloned().collect();
        for url in running {
            state.cancel_job(&url);
        }
        state.windows.clear();
    }

    #[cfg(test)]
    pub fn is_suspended(&self) -> bool {
        self.shared.state().suspended
    }

    pub fn announce(&self, settled: Settled) {
        let _ = self.shared.events.send(settled);
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

    fn start(&self, state: &mut State, job: StartJob, now: Duration, tuning: &Tuning) {
        let StartJob { key, source, distance, entering } = job;
        if state.busy || state.suspended {
            return;
        }
        if entering {
            state.ledger.counters.requested += 1;
        }
        let target = tuning.target(&source, distance);
        if let Some(job) = state.running.get(&key) {
            if !job.whole {
                job.distance.store(distance, Ordering::SeqCst);
                job.goal.send_replace(target);
            }
            if entering {
                state.ledger.counters.deduped += 1;
            }
            return;
        }
        let needed = state.ledger.needed(&key, target, now, entering);
        state.ledger.set_distance(&key, distance);
        if !needed {
            if entering {
                state.ledger.counters.deduped += 1;
            }
            return;
        }
        if !entering {
            state.ledger.counters.requested += 1;
        }
        self.launch(state, key, source, distance, false);
    }

    fn launch(&self, state: &mut State, key: String, source: Source, distance: usize, whole: bool) {
        let tuning = self.shared.tuning();
        let id = state.next_id;
        state.next_id += 1;
        let (goal, goal_receiver) = watch::channel(tuning.target(&source, distance));
        let job = Job { id, url: source.url.clone(), cancel: Arc::new(Notify::new()), active: Arc::new(AtomicBool::new(false)), goal, distance: Arc::new(AtomicUsize::new(distance)), whole };
        state.ledger.enqueue(&key);
        let live = LiveTask::new(&self.shared);
        let run = Run { key: key.clone(), source, id, cancel: job.cancel.clone(), active: job.active.clone(), distance: job.distance.clone(), goal: goal_receiver, tuning, whole };
        self.shared.handle.spawn(run_job(live, run));
        state.running.insert(key, job);
    }
}

struct StartJob {
    key: String,
    source: Source,
    distance: usize,
    entering: bool,
}

struct Run {
    key: String,
    source: Source,
    id: u64,
    cancel: Arc<Notify>,
    active: Arc<AtomicBool>,
    distance: Arc<AtomicUsize>,
    goal: watch::Receiver<usize>,
    tuning: Tuning,
    whole: bool,
}

async fn run_job(live: LiveTask, mut run: Run) {
    let shared = live.0.clone();
    shared.state().ledger.begin(&run.key);
    let mut outcome = {
        let cancel = run.cancel.clone();
        tokio::select! {
            biased;
            _ = cancel.notified() => Outcome::Cancelled,
            outcome = load(&shared, &mut run) => outcome,
        }
    };
    loop {
        {
            let mut state = shared.state();
            let promoted = state.running.get(&run.key).is_some_and(|job| job.id == run.id && job.whole);
            if run.whole || !promoted || outcome != Outcome::Loaded {
                settle(&mut state, &run.key, run.id, &outcome);
                break;
            }
            run.whole = true;
        }
        outcome = tokio::select! {
            biased;
            _ = run.cancel.notified() => Outcome::Cancelled,
            outcome = load_whole_source(&shared, &run) => outcome,
        };
    }
    drop(live);
    let _ = shared.events.send(Settled { url: run.source.url, outcome });
}

async fn load_whole_source(shared: &Arc<Shared>, run: &Run) -> Outcome {
    let gate = &shared.whole_gates[usize::from(run.source.kind == SourceKind::Image)];
    let _permit = gate.acquire(None, &run.distance, 0).await;
    mark_started(shared, &run.active);
    let outcome = load_whole(shared.loader.as_ref(), &run.source, run.tuning.timeout).await;
    if outcome == Outcome::Loaded {
        let mut state = shared.state();
        if !state.owns(&run.key, run.id) {
            return Outcome::Cancelled;
        }
        state.ledger.record_complete(&run.key);
    }
    outcome
}

async fn load(shared: &Arc<Shared>, run: &mut Run) -> Outcome {
    if run.whole {
        return load_whole_source(shared, run).await;
    }
    if is_hls(&run.source) {
        return load_chunks(shared, run).await;
    }
    let _permit = shared.gate.acquire(None, &run.distance, 0).await;
    mark_started(shared, &run.active);
    let outcome = load_unit(shared.loader.as_ref(), &run.source, run.tuning.timeout).await;
    if outcome == Outcome::Loaded {
        let mut state = shared.state();
        if !state.owns(&run.key, run.id) {
            return Outcome::Cancelled;
        }
        state.ledger.record_unit(&run.key);
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
            shared.state().ledger.step(&run.key, *run.goal.borrow())
        };
        match step {
            NextLoad::Done => return Outcome::Loaded,
            NextLoad::NeedPlan => {
                permit = Some(shared.gate.acquire(permit.take(), &run.distance, 0).await);
                mark_started(shared, &run.active);
                match read_plan(shared.loader.as_ref(), source, run.tuning.timeout, LeadingChunks::MAX_CHUNKS).await {
                    Ok(plan) => {
                        let mut state = shared.state();
                        if !state.owns(&run.key, run.id) {
                            return Outcome::Cancelled;
                        }
                        state.ledger.set_plan(&run.key, plan);
                    }
                    Err(outcome) => return outcome,
                }
            }
            NextLoad::Chunk { index, chunk, init_loaded } => {
                let room = shared.state().ledger.make_room(&run.key, run.distance.load(Ordering::SeqCst), run.tuning.budget);
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
                let (evicted, itself) = {
                    let mut state = shared.state();
                    if !state.owns(&run.key, run.id) {
                        return Outcome::Cancelled;
                    }
                    state.ledger.record_chunk(&run.key, index, parts, init, run.tuning.budget)
                };
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
        let progress = Notify::new();
        parts.push((url.clone(), bounded(timeout, &progress, loader.load(url, &source.headers, Priority::Prefetch, &progress)).await?));
        init = Some(url.clone());
    }
    let progress = Notify::new();
    parts.push((chunk.segment.clone(), bounded(timeout, &progress, loader.load(&chunk.segment, &source.headers, Priority::Prefetch, &progress)).await?));
    Ok((parts, init))
}

async fn evict(shared: &Shared, evicted: Vec<Evicted>) {
    for entry in evicted {
        for url in entry.urls {
            shared.loader.evict(&url, &entry.headers).await;
        }
    }
}

fn settle(state: &mut State, url: &str, id: u64, outcome: &Outcome) {
    if !state.owns(url, id) {
        return;
    }
    match outcome {
        Outcome::Loaded => {
            state.ledger.counters.completed += 1;
            state.wholes.remove(url);
        }
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
