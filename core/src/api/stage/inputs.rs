use super::Stage;
use crate::api::slots::SlotCall;
use crate::api::source::MediaItem;
use crate::api::types::Kind;
use crate::session::Intent;
use std::collections::HashSet;

impl Stage {
    pub fn set_window(&self, active: usize, sources: Vec<Option<MediaItem>>) {
        let keys: Vec<Option<String>> =
            sources.iter().map(|source| source.as_ref().filter(|source| source.kind != Kind::Image).map(|source| source.url.clone())).collect();
        let now = self.now();
        let commands = {
            let mut state = self.state();
            for source in sources.iter().flatten() {
                state.catalog.insert(source.url.clone(), source.clone());
            }
            let playing = state.playing_page(&keys);
            let commands = state.ring.plan(now, active, playing, &keys);
            let urls: HashSet<&str> = sources.iter().flatten().map(|source| source.url.as_str()).collect();
            let loaded: HashSet<String> = state.loaded.keys().cloned().collect();
            state.catalog.retain(|key, _| urls.contains(key.as_str()) || loaded.contains(key));
            commands
        };
        self.dispatch(commands);
        self.inner.prefetcher.set_window(active, sources.iter().map(|source| source.as_ref().map(MediaItem::for_prefetch)).collect());
    }

    pub fn set_playing(&self, key: &str, playing: bool) {
        let commands = {
            let mut state = self.state();
            if !playing {
                state.forget_lease(key)
            } else {
                let lease = state.lease_for(key);
                let orders = state.election.set_intent(key, lease, Intent::Play);
                state.commands_for(orders)
            }
        };
        self.dispatch(commands);
    }

    pub fn attach(&self, key: &str, view_token: u64) {
        let slot = {
            let mut state = self.state();
            state.views.insert(key.to_string(), view_token);
            state.loaded.get(key).map(|loaded| loaded.slot)
        };
        if let Some(slot) = slot {
            self.inner.runner.send(slot, SlotCall::Attach { view_token });
        }
    }

    pub fn detach(&self, key: &str) {
        let slot = {
            let mut state = self.state();
            state.views.remove(key);
            state.loaded.get(key).map(|loaded| loaded.slot)
        };
        if let Some(slot) = slot {
            self.inner.runner.send(slot, SlotCall::Detach);
        }
    }

    pub fn seek(&self, key: &str, position_ms: u64) {
        let slot = self.state().loaded.get(key).map(|loaded| loaded.slot);
        if let Some(slot) = slot {
            self.inner.runner.send(slot, SlotCall::Seek { position_ms });
        }
    }

    pub fn set_muted_all(&self, muted: bool) {
        let commands = self.state().ring.set_muted_all(muted);
        self.dispatch(commands);
    }

    pub fn set_muted(&self, key: &str, muted: bool) {
        let commands = self.state().ring.set_muted(key, muted);
        self.dispatch(commands);
    }

    pub fn pin(&self, key: &str) {
        let now = self.now();
        let commands = self.state().ring.pin(now, key);
        self.dispatch(commands);
    }

    pub fn unpin(&self, key: &str) {
        self.state().ring.unpin(key);
    }

    pub fn visible(&self, source: MediaItem, visible: bool) {
        let url = source.url.clone();
        let headers = source.headers.clone();
        if !visible {
            let commands = self.state().ring.untouch(&url);
            self.dispatch(commands);
            self.inner.places.retired(&url);
            return;
        }
        let now = self.now();
        let commands = {
            let mut state = self.state();
            let is_player = source.kind != Kind::Image;
            state.catalog.insert(url.clone(), source);
            if is_player { state.ring.touch(now, &url) } else { Vec::new() }
        };
        self.dispatch(commands);
        let inner = self.inner.clone();
        self.inner.handle.spawn(async move {
            let is_cached = inner.routes.is_cached(&url, &headers).await;
            inner.places.declared(&url, is_cached);
        });
    }

    pub fn release_all(&self) {
        let commands = {
            let mut state = self.state();
            let keys: Vec<String> = state.leases.keys().cloned().collect();
            for key in keys {
                state.forget_lease(&key);
            }
            state.catalog.clear();
            state.views.clear();
            state.ring.release_all()
        };
        self.dispatch(commands);
        self.inner.prefetcher.stop_all();
        self.inner.places.retire_all();
    }
}
