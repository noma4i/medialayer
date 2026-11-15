use super::gate::LoadPermit;
use super::ledger::{Evicted, NextLoad};
use super::load_source::{bounded, is_hls, load_unit, read_plan};
use super::whole::load_whole;
use crate::hls::Chunk;
use super::loader::Loader;
use crate::proxy::Priority;
use super::settled::{Outcome, Settled};
use super::source::{Source, SourceKind};
use crate::limits::LeadingChunks;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use super::runner_types::*;

pub(super) async fn run_job(live: LiveTask, mut run: Run) {
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
    #[cfg(test)]
    shared.finished.send_modify(|finished| *finished += 1);
}

pub(super) async fn load_whole_source(shared: &Arc<Shared>, run: &Run) -> Outcome {
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

pub(super) async fn load(shared: &Arc<Shared>, run: &mut Run) -> Outcome {
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

pub(super) fn mark_started(shared: &Shared, active: &AtomicBool) {
    if !active.swap(true, Ordering::SeqCst) {
        shared.state().ledger.counters.started += 1;
    }
}

pub(super) async fn load_chunks(shared: &Arc<Shared>, run: &mut Run) -> Outcome {
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

pub(super) async fn load_chunk(
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

pub(super) async fn evict(shared: &Shared, evicted: Vec<Evicted>) {
    for entry in evicted {
        for url in entry.urls {
            shared.loader.evict(&url, &entry.headers).await;
        }
    }
}

pub(super) fn settle(state: &mut PrefetchState, url: &str, id: u64, outcome: &Outcome) {
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
