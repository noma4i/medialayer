use super::{Declaration, Stage};
use crate::api::guard::AbortOnDrop;
use crate::api::Routes;
use crate::api::slots::SlotCall;
use crate::api::source::MediaItem;
use crate::api::types::Kind;
use crate::pages::Sliced;
use crate::pool::Command;
use crate::prefetch::{CacheStrategy, Source};
use crate::session::{Context, Intent};
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::sync::{Arc, PoisonError, Weak};
use super::Inner;

const MIN_RATE: f32 = 0.25;
const MAX_RATE: f32 = 4.0;

impl Stage {
    pub fn is_playing(&self, key: &str) -> bool {
        self.state().reported_playing_key().as_deref() == Some(key)
    }

    fn declare_whole<'a>(&self, sources: impl Iterator<Item = &'a MediaItem>) {
        for source in sources.filter(|source| source.strategy == CacheStrategy::Whole && source.is_remote()) {
            self.inner.prefetcher.keep_whole(source.for_prefetch());
        }
    }

    pub fn hold_wholes<'a>(&self, owner: u64, sources: impl Iterator<Item = &'a MediaItem>) {
        for source in sources.filter(|source| source.is_remote()) {
            self.inner.prefetcher.hold_whole(owner, Source { strategy: CacheStrategy::Whole, ..source.for_prefetch() });
        }
    }

    pub fn release_wholes<'a>(&self, owner: u64, sources: impl Iterator<Item = &'a MediaItem>) {
        for source in sources {
            self.inner.prefetcher.release_whole(owner, &source.for_prefetch());
        }
    }

    pub fn release_wholes_of(&self, owner: u64) {
        self.inner.prefetcher.release_owner(owner);
    }

    pub fn settled(&self) -> tokio::sync::broadcast::Receiver<crate::prefetch::Settled> {
        self.inner.prefetcher.settled()
    }

    #[cfg(test)]
    pub fn prefetch_suspended(&self) -> bool {
        self.inner.prefetcher.is_suspended()
    }

    pub fn announce_image(&self, url: &str, outcome: Result<(), String>) {
        let outcome = outcome.map_or_else(crate::prefetch::Outcome::Failed, |()| crate::prefetch::Outcome::Loaded);
        self.inner.prefetcher.announce(crate::prefetch::Settled { url: url.to_string(), outcome });
    }

    /// A place whose source names no url fails here and never reaches a player or the prefetcher.
    pub fn refuse_source(&self, key: &str) {
        self.inner.places.failed(key, super::super::source::BLANK_URL_DETAIL);
    }

    pub fn clear_refusal(&self, key: &str) {
        self.inner.places.retired(key);
    }

    pub fn replace_window(&self, active: usize, sources: impl Into<Arc<Vec<Option<MediaItem>>>>) {
        let sources = sources.into();
        self.declare_whole(sources.iter().flatten());
        self.replace_window_declared(active, sources, Vec::new());
    }

    /// `replace_window` for a caller that already handed over the whole-strategy items: the list is not walked.
    pub fn replace_window_declared(&self, active: usize, sources: Arc<Vec<Option<MediaItem>>>, wholes: Vec<MediaItem>) {
        self.declare_whole(wholes.iter());
        let mut state = self.state();
        state.window = sources;
        self.move_window_from(state, active);
    }

    pub fn move_window(&self, active: usize) {
        self.move_window_from(self.state(), active);
    }

    fn move_window_from<'a>(&'a self, mut state: super::StageGuard<'a>, active: usize) {
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
            let sources = Sliced::slice(state.window.as_slice(), reach.start.min(wanted.start)..reach.end.max(wanted.end));
            let keys = sources.map(|source| (source.kind != Kind::Image).then(|| source.url.clone()));
            for source in sources.present() {
                if !state.shown.contains_key(&source.url) {
                    state.catalog.insert(source.url.clone(), source.clone());
                }
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

    pub fn replace_image_window(&self, name: &str, active: usize, pages: impl Into<Arc<Vec<Vec<MediaItem>>>>) {
        let pages = pages.into();
        self.declare_whole(pages.iter().flatten());
        self.replace_image_window_declared(name, active, pages, Vec::new());
    }

    pub fn replace_image_window_declared(&self, name: &str, active: usize, pages: Arc<Vec<Vec<MediaItem>>>, wholes: Vec<MediaItem>) {
        self.declare_whole(wholes.iter());
        self.state().image_windows.insert(name.to_string(), pages);
        self.move_image_window(name, active);
    }

    pub fn close_image_window(&self, name: &str) {
        self.state().image_windows.remove(name);
        self.inner.prefetcher.set_image_window(name, 0, Vec::<Vec<crate::prefetch::Source>>::new());
    }

    pub fn move_image_window(&self, name: &str, active: usize) {
        let count = self.state().image_windows.get(name).map_or(0, |pages| pages.len());
        let range = self.inner.prefetcher.image_window_range(name, active, count);
        let pages = Sliced::<Vec<MediaItem>>::slice_pages(self.state().image_windows.get(name).map(|pages| pages.as_slice()).unwrap_or_default(), range);
        let remote = pages.map(|page| Some(page.iter().filter(|source| !source.is_local() && source.strategy == CacheStrategy::Window).map(MediaItem::for_prefetch).collect()));
        self.inner.prefetcher.set_image_window(name, active, remote);
    }

    pub fn set_playing(&self, key: &str, playing: bool, groups: &[Option<String>]) {
        self.apply_playing(key, playing, groups, false);
    }

    pub(crate) fn renew_playing(&self, key: &str, groups: &[Option<String>]) {
        self.apply_playing(key, true, groups, true);
    }

    fn apply_playing(&self, key: &str, playing: bool, groups: &[Option<String>], renew: bool) {
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
                orders.extend(if renew { state.election.renew_play(key, lease) } else { state.election.set_intent(key, lease, Intent::Play) });
                let commands = state.commands_for(orders);
                if !commands.iter().any(|command| matches!(command, Command::Play { .. })) {
                    crate::trace!("stage", "play dropped: {} {}", crate::trace::tail(key), state.election.blocked_reason(lease));
                }
                commands
            };
            (state, commands)
        };
        self.dispatch(state, commands);
        self.sync_foreground();
    }

    pub fn stop_all_playing(&self) {
        crate::trace!("stage", "stop_all_playing");
        let (state, commands) = {
            let mut state = self.state();
            let orders = state.election.stop_all();
            let mut commands = state.commands_for(orders);
            let keys: Vec<String> = state.leases.keys().cloned().collect();
            for key in keys {
                commands.extend(state.forget_lease(&key));
            }
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
        if is_active {
            crate::trace!("stage", "suspend {id} allow={:?}", allow_groups.unwrap_or_default());
        } else {
            crate::trace!("stage", "resume {id}");
        }
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
        let is_playing = state.is_playing(key);
        if let Some(loaded) = state.loaded.get_mut(key) {
            loaded.seeked = (!is_playing).then_some(position_ms);
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
                let is_playing = state.is_playing(key);
                if let Some(loaded) = state.loaded.get_mut(key) {
                    loaded.seeked = (!is_playing).then_some(position_ms);
                }
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
            self.inner.places.claimed(&url);
            *state.shown.entry(url.clone()).or_insert(0) += 1;
            let is_player = source.kind != Kind::Image;
            self.update_looping(&mut state, &source);
            state.catalog.insert(url.clone(), source);
            state.refresh_pending(&url);
            let commands = if is_player { state.ring.touch(now, &url) } else { Vec::new() };
            let check = self.inner.handle.spawn(check_cached(std::sync::Arc::downgrade(&self.inner), self.inner.routes.clone(), url.clone(), headers, declaration.clone()));
            state.declarations.insert(url.clone(), Declaration { token: declaration, check: Some(AbortOnDrop::new(check)), #[cfg(test)] observed: None });
            (state, commands)
        };
        self.dispatch(state, commands);
    }

    /// A later declaration of a shown key changed its item: the catalog follows without a second show.
    pub fn replace_source(&self, source: MediaItem) {
        let url = source.url.clone();
        let mut state = self.state();
        if !state.shown.contains_key(&url) {
            return;
        }
        self.update_looping(&mut state, &source);
        state.catalog.insert(url.clone(), source);
        state.refresh_pending(&url);
    }

    fn update_looping(&self, state: &mut super::StageState, source: &MediaItem) {
        if let Some(loaded) = state.loaded.get_mut(&source.url) {
            loaded.source.looping = source.looping;
            if loaded.route.looping != source.looping {
                loaded.route.looping = source.looping;
                self.inner.runner.send(loaded.slot, SlotCall::Looping { looping: source.looping });
            }
        }
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
            state.window = Arc::default();
            state.image_windows.clear();
            let commands = state.ring.release_all();
            self.inner.places.retire_all();
            (state, commands)
        };
        self.dispatch_with(state, commands, false, Vec::new());
        self.inner.prefetcher.forget_windows();
    }

    pub fn reset_players(&self) {
        let mut state = self.state();
        let commands = state.ring.release_all();
        self.dispatch(state, commands);
        for slot in 0..self.slot_count() { self.inner.runner.dispose(slot); }
        let mut state = self.state();
        let keys: Vec<String> = state.shown.keys().filter(|key| state.catalog.get(*key).is_some_and(|source| source.kind != Kind::Image)).cloned().collect();
        let mut commands = Vec::new();
        for key in keys { commands.extend(state.ring.touch(self.now(), &key)); }
        self.dispatch(state, commands);
    }
}

async fn check_cached(inner: Weak<Inner>, routes: Arc<Routes>, url: String, headers: HashMap<String, String>, declaration: Arc<()>) {
    let is_cached = routes.is_cached(&url, &headers).await;
    let Some(inner) = inner.upgrade() else { return };
    let finished = {
        let mut state = inner.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(current) = state.declarations.get_mut(&url).filter(|current| Arc::ptr_eq(&current.token, &declaration)) else {
            return;
        };
        inner.places.declared(&url, is_cached);
        current.check.take()
    };
    drop(finished);
    let stage = Stage { inner };
    stage.sync_foreground();
}
