use super::error::CacheError;
use super::evict::evict_to;
use super::inner::CacheShared;
use super::state::State;
use crate::limits::eviction_target;
use std::sync::Arc;

pub struct SpaceReservation {
    inner: Arc<CacheShared>,
    held: u64,
    written: u64,
    step: u64,
}

impl SpaceReservation {
    pub fn new(inner: Arc<CacheShared>) -> SpaceReservation {
        let budget = inner.state().limits.budget;
        let block = super::store::WRITE_CHUNK as u64;
        let step = if budget >= block { block } else { 1 };
        SpaceReservation { inner, held: 0, written: 0, step }
    }

    pub fn written(&self) -> u64 {
        self.written
    }

    pub async fn grow(&mut self, len: u64) -> Result<(), CacheError> {
        let next = self.written.checked_add(len).ok_or(CacheError::NotEnoughSpace)?;
        if next <= self.held {
            self.written = next;
            return Ok(());
        }
        let needed = next - self.held;
        let desired = needed.max(self.step);
        for attempt in 0..2 {
            let limits = {
                #[cfg(test)]
                self.inner.reservation_checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let mut state = self.inner.state();
                let limits = state.limits;
                let available = limits.budget.saturating_sub(state.used + state.retained_bytes + state.reserved);
                if available >= needed {
                    let grant = desired.min(available);
                    state.reserved += grant;
                    self.held += grant;
                    self.written = next;
                    return Ok(());
                }
                limits
            };
            if attempt == 0 {
                let shrunk = eviction_target(&limits, limits.budget).unwrap_or(limits.budget);
                let target = shrunk.min(limits.budget.saturating_sub(needed));
                let inner = self.inner.clone();
                let _ = self.inner.handle.spawn(async move { evict_to(&inner, target).await }).await;
            }
        }
        Err(CacheError::NotEnoughSpace)
    }

    pub fn settle(&mut self, state: &mut State) {
        state.reserved -= self.held;
        self.held = 0;
    }
}

impl Drop for SpaceReservation {
    fn drop(&mut self) {
        if self.held > 0 {
            self.inner.state().reserved -= self.held;
        }
    }
}
