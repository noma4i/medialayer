use super::entry::Kind;
use super::inner::{scan, CacheShared};
use super::naming::bin_path;
use super::save::mark_dirty;
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
        #[cfg(test)]
        crate::test_signal::fire();
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
    let _backlog = BacklogReset(&inner.eviction_backlog);
    let mut evicted = 0;
    let mut excluded = std::collections::HashSet::new();
    while over(inner, target) {
        let order = inner.state().order.candidates(&inner.kind_priority, YIELD_EVERY, &excluded);
        if order.is_empty() {
            break;
        }
        inner.eviction_backlog.store(order.len() as u64, Ordering::Relaxed);
        #[cfg(test)]
        crate::test_signal::fire();
        for (name, version) in order {
            if !over(inner, target) {
                break;
            }
            if evict_one(inner, &name, version).await {
                evicted += 1;
            } else {
                excluded.insert((name, version));
            }
        }
        tokio::task::yield_now().await;
    }
    if evicted > 0 {
        mark_dirty(inner);
    }
    evicted
}

async fn evict_one(inner: &Arc<CacheShared>, name: &str, version: u64) -> bool {
    let _stripe = inner.stripe(name).lock().await;
    {
        let state = inner.state();
        if state.is_protected(name) || !state.slot(name).is_some_and(|slot| slot.version == version) {
            return false;
        }
    }
    if !unlink(inner, name).await {
        return false;
    }
    let key = {
        let mut state = inner.state();
        state.evictions += 1;
        state.take(name).map(|slot| slot.key)
    };
    let listener = inner.evicted.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    if let (Some(key), Some(listener)) = (key, listener) {
        listener(&key);
    }
    true
}

pub(super) async fn unlink(inner: &CacheShared, name: &str) -> bool {
    let result = match tokio::fs::remove_file(bin_path(&inner.dir, name)).await {
        Ok(()) => true,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    };
    #[cfg(test)]
    crate::test_signal::fire();
    result
}

pub async fn remove_names(inner: &Arc<CacheShared>, names: Vec<String>) -> usize {
    let mut removed = 0;
    for (position, name) in names.iter().enumerate() {
        let _stripe = inner.stripe(name).lock().await;
        let present = inner.state().contains(name);
        if present && unlink(inner, name).await {
            inner.state().take(name);
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
