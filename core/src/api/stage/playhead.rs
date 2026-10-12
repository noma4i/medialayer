use super::{Inner, Stage};
use crate::api::slots::SlotCall;
use crate::proxy::{Playhead, PositionProbe};
use std::sync::{Arc, PoisonError, Weak};
use std::time::{Duration, Instant};

const SEEK_SLACK_MS: u64 = 1_500;
const SEEK_EXPIRY: Duration = Duration::from_secs(3);

struct StagePlayhead {
    inner: Weak<Inner>,
}

impl Playhead for StagePlayhead {
    fn position_ms(&self, playlist: &str) -> PositionProbe {
        let inner = self.inner.upgrade();
        let playlist = playlist.to_string();
        Box::pin(async move { Stage { inner: inner? }.position_of(&playlist).await })
    }

    fn readahead(&self, playlist: &str, hint_ms: u64) {
        if let Some(inner) = self.inner.upgrade() {
            Stage { inner }.send_readahead(playlist, hint_ms);
        }
    }
}

impl Stage {
    pub(super) fn playhead(&self) -> Arc<dyn Playhead> {
        Arc::new(StagePlayhead { inner: Arc::downgrade(&self.inner) })
    }

    pub(super) fn note_seek(&self, url: &str, position_ms: u64) {
        self.inner.seeks.lock().unwrap_or_else(PoisonError::into_inner).insert(url.to_string(), (position_ms, Instant::now()));
        self.inner.routes.nudge_window(url);
    }

    pub(super) fn url_of(&self, key: &str) -> Option<String> {
        self.state().loaded.get(key).map(|loaded| loaded.source.url.clone())
    }

    async fn position_of(&self, url: &str) -> Option<u64> {
        let slots: Vec<usize> = self.state().loaded.values().filter(|loaded| loaded.source.url == url).map(|loaded| loaded.slot).collect();
        let mut position = None;
        for slot in slots {
            if let Some(sample) = self.inner.runner.sample(slot).await {
                position = position.max(Some(sample.position_ms));
            }
        }
        let position = position?;
        let mut seeks = self.inner.seeks.lock().unwrap_or_else(PoisonError::into_inner);
        match seeks.get(url).copied() {
            Some((target, at)) if at.elapsed() < SEEK_EXPIRY && position + SEEK_SLACK_MS < target => Some(target),
            Some(_) => {
                seeks.remove(url);
                Some(position)
            }
            None => Some(position),
        }
    }

    fn send_readahead(&self, url: &str, hint_ms: u64) {
        let slots: Vec<usize> = self.state().loaded.values().filter(|loaded| loaded.source.url == url).map(|loaded| loaded.slot).collect();
        for slot in slots {
            self.inner.runner.send(slot, SlotCall::Readahead { ms: hint_ms });
        }
    }
}
