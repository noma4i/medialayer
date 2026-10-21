use super::entry::{Entry, Kind};
use super::inner::{scan, CacheShared};
use super::naming::bin_path;
use super::policy::victim_order;
use super::save::mark_dirty;
use super::state::Item;
use crate::limits::eviction_target;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const YIELD_EVERY: usize = 128;

pub async fn evict_if_needed(inner: &Arc<CacheShared>) -> usize {
    let target = {
        let state = inner.state();
        eviction_target(&state.limits, state.used + state.retained_bytes)
    };
    match target {
        Some(target) => evict_to(inner, target).await,
        None => 0,
    }
}

pub fn trigger_eviction(inner: &Arc<CacheShared>) {
    let needed = {
        let state = inner.state();
        eviction_target(&state.limits, state.used + state.retained_bytes).is_some()
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

struct BacklogReset<'a>(&'a std::sync::atomic::AtomicU64);

impl Drop for BacklogReset<'_> {
    fn drop(&mut self) {
        self.0.store(0, Ordering::Relaxed);
    }
}

fn over(inner: &CacheShared, target: u64) -> bool {
    let state = inner.state();
    state.used + state.retained_bytes + state.reserved > target
}

pub async fn evict_to(inner: &Arc<CacheShared>, target: u64) -> usize {
    if !over(inner, target) {
        return 0;
    }
    let candidates = scan(inner).await;
    let (default, now) = (inner.state().limits.strategy, inner.now());
    let (kept, others): (Vec<_>, Vec<_>) = candidates.into_iter().filter(|item| !item.protected).partition(|item| item.kept);
    let pairs = |items: Vec<Item>| -> Vec<(String, Entry)> { items.into_iter().map(|item| (item.name, item.entry)).collect() };
    let (others, kept) = (pairs(others), pairs(kept));
    let overrides = inner.overrides.clone();
    let priority = inner.kind_priority;
    let ordered = tokio::task::spawn_blocking(move || {
        let mut order = victim_order(others, default, &overrides, &priority, now);
        order.extend(victim_order(kept, default, &overrides, &priority, now));
        order
    })
    .await;
    let Ok(order) = ordered else { return 0 };
    inner.eviction_backlog.store(order.len() as u64, Ordering::Relaxed);
    let _backlog = BacklogReset(&inner.eviction_backlog);
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
    if evicted > 0 {
        mark_dirty(inner);
    }
    evicted
}

async fn evict_one(inner: &Arc<CacheShared>, name: &str) -> bool {
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

pub async fn remove_names(inner: &Arc<CacheShared>, names: Vec<String>) -> usize {
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

pub async fn names_where(inner: &CacheShared, wanted: impl Fn(&str, Kind) -> bool) -> Vec<String> {
    scan(inner).await.into_iter().filter(|item| wanted(&item.key, item.entry.kind)).map(|item| item.name).collect()
}
