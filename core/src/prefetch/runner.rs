use super::gate::Gate;
use super::ledger::{Counters, PrefetchLedger};
use super::loader::Loader;
use super::settled::{Outcome, Settled};
use super::source::Source;
use super::strategy::CacheStrategy;
use super::window::{diff_window, wanted_jobs, wanted_page_jobs, PageWindow, PrefetchJob};
use crate::cache::Clock;
use crate::limits::Limits;
use crate::pages::Sliced;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch, Notify};
use super::runner_types::*;
use super::runner_load::run_job;

#[derive(Clone)]
pub struct Prefetcher {
    shared: Arc<Shared>,
    _owner: Arc<PrefetchOwner>,
}

impl Prefetcher {
    pub fn new(loader: Arc<dyn Loader>, limits: &Limits, ledger: PrefetchLedger, clock: Arc<dyn Clock>) -> Prefetcher {
        let shared = Shared {
            failures: Mutex::new(Arc::default()),
            loader,
            tuning: Mutex::new(Tuning::from_limits(limits)),
            image_windows: Mutex::new(HashMap::new()),
            gate: Gate::new(limits.max_concurrent as usize),
            whole_gates: [Gate::new(WHOLE_MEDIA_PARALLEL), Gate::new(WHOLE_IMAGE_PARALLEL)],
            clock,
            state: Mutex::new(PrefetchState { ledger, windows: HashMap::new(), last: HashMap::new(), running: HashMap::new(), wholes: HashMap::new(), mixed: HashSet::new(), next_id: 0, busy: false, suspended: false }),
            live: AtomicUsize::new(0),
            events: broadcast::channel(EVENT_CAPACITY).0,
            #[cfg(test)]
            finished: watch::channel(0).0,
            handle: Handle::current(),
        };
        let shared = Arc::new(shared);
        Prefetcher { _owner: Arc::new(PrefetchOwner(shared.clone())), shared }
    }

    pub fn set_limits(&self, limits: &Limits) {
        *self.shared.tuning.lock().unwrap_or_else(PoisonError::into_inner) = Tuning::from_limits(limits);
        self.shared.gate.set_limit(limits.max_concurrent as usize);
        let window = Duration::from_secs(limits.dedupe_window_seconds.into());
        self.shared.state().ledger.set_limits(window, limits.dedupe_max_entries as usize);
    }

    pub fn set_failures(&self, failures: Arc<crate::failures::Failures>) {
        *self.shared.failures.lock().unwrap_or_else(PoisonError::into_inner) = failures;
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

    pub fn eviction_listener(&self) -> Arc<dyn Fn(&str) + Send + Sync> {
        let shared = Arc::downgrade(&self.shared);
        Arc::new(move |key: &str| {
            let Some(shared) = shared.upgrade() else { return };
            let source = key.split('#').next().unwrap_or(key);
            let mut state = shared.state();
            if !state.running.values().any(|job| job.url == source) {
                state.ledger.forget_loaded(Some(source));
            }
        })
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

    pub fn retry(&self, source: &Source) {
        let key = self.shared.loader.cache_key(&source.url, &source.headers);
        let tuning = self.shared.tuning();
        let now = Duration::from_millis(self.shared.clock.now_ms());
        let mut state = self.shared.state();
        let wanted: Vec<_> = state.last.iter().filter_map(|(window, (jobs, sources))| {
            let job = jobs.iter().find(|job| job.key == key)?.clone();
            Some((window.clone(), job, sources.get(&key)?.clone()))
        }).collect();
        if let Some(whole) = state.wholes.get(&key).map(|held| held.source.clone()) {
            self.start_whole(&mut state, key.clone(), whole, now, true);
        }
        for (window, job, source) in wanted {
            let jobs = state.windows.entry(window).or_default();
            if !jobs.iter().any(|held| held.key == key) { jobs.push(job.clone()); }
            self.start(&mut state, StartJob { key: key.clone(), source, distance: job.priority.unsigned_abs() as usize, entering: true }, now, &tuning);
        }
    }

    /// A place the user sees holds its source past the entry limit: the limit caps preload, a shown place never fails on it.
    pub fn keep_shown_whole(&self, source: Source) {
        self.admit_whole(WINDOW_OWNER, source, true);
    }

    /// The place left the screen: its queued whole goes back behind the places that are still seen.
    pub fn unshow_whole(&self, source: &Source) {
        let key = self.shared.loader.cache_key(&source.url, &source.headers);
        let state = self.shared.state();
        if let Some(job) = state.running.get(&key) {
            job.distance.store(usize::MAX, Ordering::SeqCst);
        }
    }

    /// A place shows a url that a whole hold also holds, with another strategy: one trace line per url, the load itself is shared by key.
    pub fn note_shown_strategy(&self, source: &Source) {
        if source.strategy == CacheStrategy::Whole {
            return;
        }
        let key = self.shared.loader.cache_key(&source.url, &source.headers);
        let mut state = self.shared.state();
        if state.wholes.contains_key(&key) && state.mixed.insert(key) {
            crate::trace!("prefetch", "mixed strategy {} shown as {:?} while held whole", crate::trace::tail(&source.url), source.strategy);
        }
    }

    pub fn hold_whole(&self, owner: u64, source: Source) {
        self.admit_whole(owner, source, false);
    }

    fn admit_whole(&self, owner: u64, source: Source, shown: bool) {
        if source.strategy != CacheStrategy::Whole {
            return;
        }
        let now = Duration::from_millis(self.shared.clock.now_ms());
        let key = self.shared.loader.cache_key(&source.url, &source.headers);
        let mut state = self.shared.state();
        if !shown && !state.wholes.contains_key(&key) && state.wholes.keys().filter(|held| state.running.contains_key(*held)).count() >= self.shared.tuning().max_entries {
            state.ledger.counters.requested += 1;
            state.ledger.counters.failed += 1;
            drop(state);
            let reason = "whole queue is full".to_string();
            let kind = match source.kind {
                super::source::SourceKind::Image => crate::api::Kind::Image,
                super::source::SourceKind::Video => crate::api::Kind::Video,
                super::source::SourceKind::Audio => crate::api::Kind::Audio,
            };
            let failures = self.shared.failures.lock().unwrap_or_else(PoisonError::into_inner).clone();
            failures.report(crate::failures::MediaFailure { kind, url: source.url.clone(), stage: crate::failures::FailureStage::Load, reason: reason.clone() });
            let _ = self.shared.events.send(Settled { url: source.url, outcome: Outcome::Failed(reason) });
            return;
        }
        state.wholes.entry(key.clone()).or_insert_with(|| WholeHold { source: source.clone(), owners: HashSet::new() }).owners.insert(owner);
        self.start_whole(&mut state, key.clone(), source, now, true);
        if shown {
            if let Some(job) = state.running.get(&key) {
                job.distance.store(0, Ordering::SeqCst);
            }
        }
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

    fn let_go(state: &mut PrefetchState, owner: u64, key: &str) {
        let Some(held) = state.wholes.get_mut(key) else { return };
        held.owners.remove(&owner);
        if !held.owners.is_empty() {
            return;
        }
        state.wholes.remove(key);
        state.mixed.remove(key);
        if state.running.get(key).is_some_and(|job| job.whole) && !state.is_wanted(key) {
            state.cancel_job(key);
            state.ledger.set_distance(key, usize::MAX);
        }
    }

    fn start_whole(&self, state: &mut PrefetchState, key: String, source: Source, now: Duration, entering: bool) {
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

    pub(super) fn cancel_everything(shared: &Shared) {
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
        #[cfg(test)]
        crate::test_signal::fire();
    }

    pub fn settled(&self) -> broadcast::Receiver<Settled> {
        self.shared.events.subscribe()
    }

    #[cfg(test)]
    pub async fn wait_finished(&self, count: usize) {
        assert!(self.shared.finished.subscribe().wait_for(|finished| *finished >= count).await.is_ok());
    }

    #[cfg(test)]
    pub async fn wait_queued(&self, count: usize) {
        self.shared.gate.wait_queued(count).await;
    }

    #[cfg(test)]
    pub async fn wait_whole_queued(&self, image: bool, count: usize) {
        self.shared.whole_gates[usize::from(image)].wait_queued(count).await;
    }

    #[cfg(test)]
    pub async fn wait_live_at_most(&self, count: usize) {
        let mut finished = self.shared.finished.subscribe();
        loop {
            finished.borrow_and_update();
            if self.live_tasks() <= count { return; }
            assert!(finished.changed().await.is_ok());
        }
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

    fn start(&self, state: &mut PrefetchState, job: StartJob, now: Duration, tuning: &Tuning) {
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

    fn launch(&self, state: &mut PrefetchState, key: String, source: Source, distance: usize, whole: bool) {
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
