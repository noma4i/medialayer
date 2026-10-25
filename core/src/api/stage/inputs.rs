use super::Stage;
use crate::api::slots::SlotCall;
use crate::api::source::MediaItem;
use crate::api::types::Kind;
use crate::pages::Sliced;
use crate::prefetch::CacheStrategy;
use crate::session::{Context, Intent};
use std::collections::BTreeSet;
use std::sync::PoisonError;

const MIN_RATE: f32 = 0.25;
const MAX_RATE: f32 = 4.0;

impl Stage {
    fn declare_whole<'a>(&self, sources: impl Iterator<Item = &'a MediaItem>) {
        for source in sources.filter(|source| source.strategy == CacheStrategy::Whole && !source.is_local()) {
            self.inner.prefetcher.keep_whole(source.for_prefetch());
        }
    }

    pub fn keep_whole(&self, source: &MediaItem) {
        self.declare_whole(std::iter::once(source));
    }

    pub fn replace_window(&self, active: usize, sources: Vec<Option<MediaItem>>) {
        self.declare_whole(sources.iter().flatten());
        let mut state = self.state();
        state.window = sources;
        self.move_window_from(state, active);
    }

    pub fn move_window(&self, active: usize) {
        self.move_window_from(self.state(), active);
    }

    fn move_window_from<'a>(&'a self, mut state: super::StateGuard<'a>, active: usize) {
        let playing = if state.window.is_empty() { state.playing_slots() } else { Vec::new() };
        if !playing.is_empty() {
            drop(state);
            let positions: Vec<(usize, String, u64, u64)> = playing
                .into_iter()
                .filter_map(|(slot, key, generation)| Some((slot, key, generation, self.inner.runner.sample_now(slot)?.position_ms)))
                .filter(|(_, _, _, position)| *position > 0)
                .collect();
            state = self.state();
            for (slot, key, generation, position) in positions {
                if state.is_loaded_as(&key, slot, generation) {
                    state.positions.remember(&key, position);
                }
            }
        }
        let (count, reach) = (state.window.len(), state.ring.pages(active, state.window.len()));
        crate::trace!("stage", "window active={active} count={count}");
        let wanted = self.inner.prefetcher.window_range(active, count);
        let now = self.now();
        let (state, commands, sources) = {
            let sources = Sliced::slice(&state.window, reach.start.min(wanted.start)..reach.end.max(wanted.end));
            let keys = sources.map(|source| (source.kind != Kind::Image).then(|| source.url.clone()));
            for source in sources.present() {
                state.catalog.insert(source.url.clone(), source.clone());
                state.refresh_pending(&source.url);
            }
            let playing = state.playing_page(&keys, reach.start..reach.end);
            let commands = state.ring.plan(now, active, playing, &keys);
            let sliced = sources.present().map(|source| source.url.clone()).collect();
            for departed in std::mem::replace(&mut state.sliced, sliced) {
                state.retire(&departed);
            }
            (state, commands, sources)
        };
        self.dispatch(state, commands);
        let remote = sources.map(|source| (!source.is_local() && source.strategy == CacheStrategy::Window).then(|| source.for_prefetch()));
        self.inner.prefetcher.set_window(active, remote);
    }

    pub fn replace_image_window(&self, name: &str, active: usize, pages: Vec<Vec<MediaItem>>) {
        self.declare_whole(pages.iter().flatten());
        self.state().image_windows.insert(name.to_string(), pages);
        self.move_image_window(name, active);
    }

    pub fn move_image_window(&self, name: &str, active: usize) {
        let count = self.state().image_windows.get(name).map_or(0, Vec::len);
        let range = self.inner.prefetcher.image_window_range(name, active, count);
        let pages = Sliced::<Vec<MediaItem>>::slice_pages(self.state().image_windows.get(name).map(Vec::as_slice).unwrap_or_default(), range);
        let remote = pages.map(|page| Some(page.iter().filter(|source| !source.is_local() && source.strategy == CacheStrategy::Window).map(MediaItem::for_prefetch).collect()));
        self.inner.prefetcher.set_image_window(name, active, remote);
    }

    pub fn set_playing(&self, key: &str, playing: bool, groups: &[Option<String>]) {
        crate::trace!("stage", "set_playing {} {playing}", crate::trace::tail(key));
        let (state, commands) = {
            let mut state = self.state();
            let commands = if !playing {
                state.forget_lease(key)
            } else {
                let lease = state.lease_for(key);
                let default = Context::Ring.policy().groups;
                let mut groups: BTreeSet<String> = groups.iter().flat_map(|group| group.as_deref().map_or_else(|| default.clone(), |group| BTreeSet::from([group.trim().to_string()]))).collect();
                if groups.is_empty() {
                    groups = default;
                }
                let mut orders = state.election.set_groups(key, lease, groups);
                orders.extend(state.election.set_intent(key, lease, Intent::Play));
                state.commands_for(orders)
            };
            (state, commands)
        };
        self.dispatch(state, commands);
        self.sync_foreground();
    }

    pub fn suspend(&self, id: u64, allow_groups: &[String]) {
        let groups: Vec<&str> = allow_groups.iter().map(String::as_str).collect();
        self.set_suspension(id, true, Some(&groups));
    }

    pub fn resume(&self, id: u64) {
        self.set_suspension(id, false, None);
    }

    fn set_suspension(&self, id: u64, is_active: bool, allow_groups: Option<&[&str]>) {
        let mut state = self.state();
        let orders = state.election.set_suspension(id, is_active, allow_groups).unwrap_or_default();
        let commands = state.commands_for(orders);
        self.dispatch(state, commands);
    }

    pub fn set_peeking(&self, key: &str, peeking: bool) {
        crate::trace!("stage", "set_peeking {} {peeking}", crate::trace::tail(key));
        let mut state = self.state();
        let commands = state.ring.set_peeking(key, peeking);
        self.dispatch(state, commands);
    }

    pub fn attach(&self, key: &str, view_token: u64) {
        let mut state = self.state();
        state.views.insert(key.to_string(), view_token);
        let slot = state.loaded.get(key).map(|loaded| loaded.slot);
        if let Some(slot) = slot {
            self.inner.runner.send(slot, SlotCall::Attach { view_token });
        }
    }

    pub fn detach(&self, key: &str) {
        let mut state = self.state();
        state.views.remove(key);
        state.retire(key);
        let slot = state.loaded.get(key).map(|loaded| loaded.slot);
        if let Some(slot) = slot {
            self.inner.runner.send(slot, SlotCall::Detach);
        }
    }

    pub fn seek(&self, key: &str, position_ms: u64) {
        let mut state = self.state();
        if let Some(loaded) = state.loaded.get(key) {
            self.note_seek(&loaded.source.url, position_ms);
            self.inner.runner.send(loaded.slot, SlotCall::Seek { position_ms });
        } else if position_ms > 0 {
            state.positions.remember(key, position_ms);
        } else {
            state.positions.forget(key);
        }
    }

    pub fn seek_fraction(&self, key: &str, fraction: f64) {
        if !fraction.is_finite() {
            return;
        }
        let fraction = fraction.clamp(0.0, 1.0);
        let load = self.state().loaded.get(key).map(|loaded| (loaded.slot, loaded.generation));
        let duration = load.and_then(|(slot, _)| self.inner.runner.sample_now(slot)).and_then(|sample| sample.duration_ms).filter(|duration| *duration > 0);
        let mut state = self.state();
        let current = state.loaded.get(key).map(|loaded| (loaded.slot, loaded.generation));
        match (current, duration) {
            (Some((slot, _)), Some(duration)) if current == load => {
                state.positions.forget(key);
                let position_ms = (duration as f64 * fraction) as u64;
                self.note_seek(&state.loaded[key].source.url, position_ms);
                self.inner.runner.send(slot, SlotCall::Seek { position_ms });
            }
            _ => state.positions.remember_fraction(key, fraction),
        }
    }

    pub fn set_rate(&self, key: &str, rate: f32) {
        if !rate.is_finite() {
            return;
        }
        let rate = rate.clamp(MIN_RATE, MAX_RATE);
        let mut state = self.state();
        if rate == 1.0 {
            state.rates.remove(key);
        } else {
            state.rates.insert(key.to_string(), rate);
        }
        let slot = state.loaded.get(key).map(|loaded| loaded.slot);
        if let Some(slot) = slot {
            self.inner.runner.send(slot, SlotCall::Rate { rate });
        }
    }

    pub fn set_muted_all(&self, muted: bool) {
        let mut state = self.state();
        let commands = state.ring.set_muted_all(muted);
        self.dispatch(state, commands);
    }

    pub fn set_muted(&self, key: &str, muted: bool) {
        let mut state = self.state();
        let commands = state.ring.set_muted(key, muted);
        self.dispatch(state, commands);
    }

    pub fn pin(&self, key: &str) {
        let now = self.now();
        let mut state = self.state();
        let commands = state.ring.pin(now, key);
        self.dispatch(state, commands);
    }

    pub fn unpin(&self, key: &str) {
        self.state().ring.unpin(key);
    }

    pub fn visible(&self, source: MediaItem, visible: bool) {
        let url = source.url.clone();
        let headers = source.headers.clone();
        crate::trace!("stage", "visible {} {visible}", crate::trace::tail(&url));
        if !visible {
            let mut state = self.state();
            if state.leave(&url) {
                return;
            }
            state.declarations.remove(&url);
            state.retire(&url);
            let commands = state.ring.untouch(&url);
            self.inner.places.retired(&url);
            self.dispatch(state, commands);
            self.forget_preferences(&url);
            return;
        }
        let now = self.now();
        self.declare_whole(std::iter::once(&source));
        let declaration = std::sync::Arc::new(());
        let (state, commands) = {
            let mut state = self.state();
            state.declarations.insert(url.clone(), declaration.clone());
            self.inner.places.claimed(&url);
            *state.shown.entry(url.clone()).or_insert(0) += 1;
            let is_player = source.kind != Kind::Image;
            state.catalog.insert(url.clone(), source);
            state.refresh_pending(&url);
            let commands = if is_player { state.ring.touch(now, &url) } else { Vec::new() };
            (state, commands)
        };
        self.dispatch(state, commands);
        let inner = self.inner.clone();
        self.inner.handle.spawn(async move {
            let is_cached = inner.routes.is_cached(&url, &headers).await;
            let stage = Stage { inner: inner.clone() };
            {
                let state = stage.state();
                if state.declarations.get(&url).is_some_and(|current| std::sync::Arc::ptr_eq(current, &declaration)) {
                    inner.places.declared(&url, is_cached);
                }
            }
            stage.sync_foreground();
            #[cfg(test)]
            inner.declaration_completed.notify_one();
        });
    }

    pub fn release_all(&self) {
        let (state, commands) = {
            let mut state = self.state();
            let keys: Vec<String> = state.leases.keys().cloned().collect();
            for key in keys {
                state.forget_lease(&key);
            }
            state.catalog.clear();
            state.sliced.clear();
            self.inner.seeks.lock().unwrap_or_else(PoisonError::into_inner).clear();
            state.pending_wanted.clear();
            state.views.clear();
            state.shown.clear();
            state.declarations.clear();
            state.rates.clear();
            state.positions.clear();
            state.releases += 1;
            state.window.clear();
            state.image_windows.clear();
            let commands = state.ring.release_all();
            self.inner.places.retire_all();
            (state, commands)
        };
        self.dispatch_with(state, commands, false, Vec::new());
        self.inner.prefetcher.forget_windows();
    }
}
