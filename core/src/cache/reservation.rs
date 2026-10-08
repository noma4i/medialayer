use super::error::CacheError;
use super::evict::evict_to;
use super::inner::CacheShared;
use super::state::State;
use crate::limits::eviction_target;
use std::sync::Arc;

pub struct SpaceReservation {
    inner: Arc<CacheShared>,
    held: u64,
}

impl SpaceReservation {
    pub fn new(inner: Arc<CacheShared>) -> SpaceReservation {
        SpaceReservation { inner, held: 0 }
    }

    pub fn held(&self) -> u64 {
        self.held
    }

    pub async fn grow(&mut self, len: u64) -> Result<(), CacheError> {
        for attempt in 0..2 {
            let limits = {
                let mut state = self.inner.state();
                let limits = state.limits;
                if state.used + state.reserved + len <= limits.budget {
                    state.reserved += len;
                    self.held += len;
                    return Ok(());
                }
                limits
            };
            if attempt == 0 {
                let shrunk = eviction_target(&limits, limits.budget).unwrap_or(limits.budget);
                evict_to(&self.inner, shrunk.min(limits.budget.saturating_sub(len))).await;
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
