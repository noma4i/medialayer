use super::entry::Kind;
use super::error::CacheError;
use super::inner::{scan, Inner};
use super::naming::bin_path;
use super::policy::victim_order;
use super::save::mark_dirty;
use crate::limits::eviction_target;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const YIELD_EVERY: usize = 128;

pub async fn evict_if_needed(inner: &Arc<Inner>) -> usize {
    let target = {
        let state = inner.state();
        eviction_target(&state.limits, state.used)
    };
    match target {
        Some(target) => evict_to(inner, target).await,
        None => 0,
    }
}

pub fn trigger_eviction(inner: &Arc<Inner>) {
    let needed = {
        let state = inner.state();
        eviction_target(&state.limits, state.used).is_some()
    };
    if !needed || inner.evicting.swap(true, Ordering::SeqCst) {
        return;
    }
    let task_inner = inner.clone();
    inner.handle.spawn(async move {
        let evicted = evict_if_needed(&task_inner).await;
        task_inner.evicting.store(false, Ordering::SeqCst);
        if evicted > 0 {
            trigger_eviction(&task_inner);
        }
    });
}

fn over(inner: &Inner, target: u64) -> bool {
    let state = inner.state();
    state.used + state.reserved > target
}

pub async fn evict_to(inner: &Arc<Inner>, target: u64) -> usize {
    if !over(inner, target) {
        return 0;
    }
    let candidates = scan(inner).await;
    let (default, now) = (inner.state().limits.strategy, inner.now());
    let pairs = candidates.into_iter().filter(|item| !item.protected).map(|item| (item.name, item.entry)).collect();
    let overrides = inner.overrides.clone();
    let priority = inner.kind_priority;
    let ordered = tokio::task::spawn_blocking(move || victim_order(pairs, default, &overrides, &priority, now)).await;
    let Ok(order) = ordered else { return 0 };
    inner.eviction_backlog.store(order.len() as u64, Ordering::Relaxed);
    let mut evicted = 0;
    for (position, name) in order.iter().enumerate() {
        if !over(inner, target) {
            break;
        }
        if evict_one(inner, name).await {
            evicted += 1;
        }
        inner.eviction_backlog.store((order.len() - position - 1) as u64, Ordering::Relaxed);
        if (position + 1).is_multiple_of(YIELD_EVERY) {
            tokio::task::yield_now().await;
        }
    }
    inner.eviction_backlog.store(0, Ordering::Relaxed);
    if evicted > 0 {
        mark_dirty(inner);
    }
    evicted
}

async fn evict_one(inner: &Arc<Inner>, name: &str) -> bool {
    let _stripe = inner.stripe(name).lock().await;
    {
        let mut state = inner.state();
        if state.is_protected(name) || state.take(name).is_none() {
            return false;
        }
        state.evictions += 1;
    }
    let _ = tokio::fs::remove_file(bin_path(&inner.dir, name)).await;
    true
}

pub async fn reserve(inner: &Arc<Inner>, name: &str, size: u64) -> Result<(), CacheError> {
    for attempt in 0..2 {
        let (replaced, limits) = {
            let mut state = inner.state();
            let (replaced, limits) = (state.size_of(name), state.limits);
            if size > limits.budget {
                return Err(CacheError::NotEnoughSpace);
            }
            if state.used + state.reserved - replaced + size <= limits.budget {
                state.reserved += size;
                return Ok(());
            }
            (replaced, limits)
        };
        if attempt == 0 {
            let shrunk = eviction_target(&limits, limits.budget).unwrap_or(limits.budget);
            evict_to(inner, shrunk.min(limits.budget - size) + replaced).await;
        }
    }
    Err(CacheError::NotEnoughSpace)
}

pub fn release(inner: &Inner, size: u64) {
    inner.state().reserved -= size;
}

pub async fn remove_names(inner: &Arc<Inner>, names: Vec<String>) -> usize {
    let mut removed = 0;
    for (position, name) in names.iter().enumerate() {
        let _stripe = inner.stripe(name).lock().await;
        let taken = inner.state().take(name).is_some();
        if taken {
            let _ = tokio::fs::remove_file(bin_path(&inner.dir, name)).await;
            removed += 1;
        }
        if (position + 1).is_multiple_of(YIELD_EVERY) {
            tokio::task::yield_now().await;
        }
    }
    if removed > 0 {
        mark_dirty(inner);
    }
    removed
}

pub async fn names_where(inner: &Inner, wanted: impl Fn(&str, Kind) -> bool) -> Vec<String> {
    scan(inner).await.into_iter().filter(|item| wanted(&item.key, item.entry.kind)).map(|item| item.name).collect()
}
