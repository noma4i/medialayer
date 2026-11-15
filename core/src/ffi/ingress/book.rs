use super::command::{Answer, Declared, Finished, OwnerKind, PlaceInput, PlayerEvent, Query};
use super::queue::{InputQueue, Reserve, EntryPlace};
use crate::api::{placeholder_for, AbortOnDrop, MediaError, MediaItem, MediaState};
use tokio::runtime::Handle;
use super::super::assembled::Assembled;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use super::book_keys::*;
pub(crate) use super::book_types::Book;
use super::book_types::*;

impl Book {
    pub fn new(assembled: Arc<Assembled>, queue: Arc<InputQueue>, handle: Handle) -> Book {
        Book { assembled, queue, handle, seq: 0, places: HashMap::new(), keys: HashMap::new(), windows: HashMap::new(), video_order: BTreeMap::new(), video_applied: None, subscriptions: HashMap::new(), suspensions: HashMap::new(), calls: HashMap::new(), declarations: HashMap::new() }
    }

    pub(super) fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    pub fn apply(&mut self, command: Declared, reserve: Reserve, slot: EntryPlace) {
        match super::clean::declared(command) {
            Declared::PlaceOpen { owner, item } => self.open_place(owner, item, reserve),
            Declared::PlaceUpdate { owner, input } => self.update_place(owner, input, reserve),
            Declared::PlaceSeek { owner, fraction } => {
                if let Some(place) = self.places.get(&owner) {
                    self.assembled.stage.seek_fraction(&place.item.url, fraction);
                }
            }
            Declared::KeySeek { key, fraction } => self.assembled.stage.seek_fraction(&key, fraction),
            Declared::PlaceState { owner, state } => {
                if let Some(place) = self.places.get(&owner) {
                    if state == MediaState::Ready {
                        self.assembled.tones.refresh(&place.item.url);
                    }
                    self.assembled.places.item_state(&place.item.url, state);
                }
            }
            Declared::WindowOpen { owner, kind } => self.open_window(owner, kind, reserve),
            Declared::WindowActive { owner, is_active } => self.set_window_active(owner, is_active),
            Declared::WindowReady { owner, listener } => self.window_ready(owner, listener),
            Declared::VideoWindow { owner, active, sources } => self.video_window(owner, active, sources, reserve),
            Declared::ImageWindow { owner, active, pages } => self.image_window(owner, active, pages, reserve),
            Declared::Suspend { owner, groups } => {
                self.assembled.stage.suspend(owner, &groups);
                self.suspensions.insert(owner, reserve);
            }
            Declared::CallHold { owner } => self.hold_call(owner, reserve),
            Declared::WholeOpen { owner } => {
                self.declarations.insert(owner, reserve);
            }
            Declared::WholeAdd { owner, items } => self.assembled.stage.hold_wholes(owner, items.iter()),
            Declared::WholeRemove { owner, items } => self.assembled.stage.release_wholes(owner, items.iter()),
            Declared::Subscribe { owner, subscribing } => self.subscribe(owner, subscribing, reserve),
            Declared::Player(event) => self.player(event),
            Declared::Lifecycle { foreground } => {
                if foreground { self.assembled.signals.foreground() } else { self.assembled.signals.background() }
            }
            Declared::Memory(level) => self.assembled.signals.memory_warning(level),
            Declared::Preparation(preparation) => self.assembled.tones.set_preparation(preparation),
            Declared::MutedAll(muted) => self.assembled.stage.set_muted_all(muted),
            Declared::ReleaseAll => self.release_all(),
            Declared::Release { .. } => {}
            Declared::Query { query, reply } => {
                let answer = self.answer(query, reserve, slot);
                let _ = reply.send(answer);
            }
        }
    }

    /// Fixes the logical part of the answer here, behind every earlier declaration; any IO starts from that point as owned work.
    pub(super) fn answer(&self, query: Query, reserve: Reserve, slot: EntryPlace) -> Answer {
        let assembled = self.assembled.clone();
        match query {
            Query::PlayUrl(_) if !self.calls.is_empty() => Answer::Failed(MediaError::InCall),
            Query::PlayUrl(item) => Answer::Route(assembled.routes.play_url(&item)),
            Query::Refresh(item, route) => Answer::Route(assembled.routes.refresh(&item, &route)),
            Query::Pool => Answer::Pool(assembled.pool_stats()),
            Query::Placeholder(item, ready) => Answer::Placeholder(placeholder_for(item, ready)),
            Query::Cached(item) => self.running(reserve, slot, async move { Finished::Cached(assembled.routes.is_cached(&item.url, &item.headers).await) }),
            Query::Tone(item, band) => self.running(reserve, slot, async move { Finished::Tone(assembled.tones.tone(&item, band).await) }),
            Query::Pixels(item) => self.running(reserve, slot, async move { Finished::Pixels(assembled.tones.pixels_needed(&item).await) }),
            Query::ImageSize { url, headers } => self.running(reserve, slot, async move { Finished::ImageSize(assembled.routes.image_size(&url, &headers).await) }),
            Query::Decode(bytes) => self.running(reserve, slot, async move { Finished::Decode(assembled.routes.reserve_decode(bytes).await) }),
            Query::Capture(bytes) => self.running(reserve, slot, async move { Finished::Capture(assembled.routes.reserve_capture(bytes).await) }),
            Query::Image { url, headers } => self.running(reserve, slot, async move {
                let result = assembled.routes.image_bytes(&url, &headers, crate::memory::InputLimits::default().response_bytes).await;
                assembled.stage.announce_image(&url, result.as_ref().map(|_| ()).map_err(ToString::to_string));
                Finished::Image(result)
            }),
        }
    }

    /// The reservation and the entry place move into the work and are freed when it ends or is dropped, never earlier.
    pub(super) fn running(&self, reserve: Reserve, slot: EntryPlace, work: impl std::future::Future<Output = Finished> + Send + 'static) -> Answer {
        Answer::Running(AbortOnDrop::new(self.handle.spawn(async move {
            let _held = (reserve, slot);
            work.await
        })))
    }

    pub(super) fn player(&self, event: PlayerEvent) {
        let stage = &self.assembled.stage;
        match event {
            PlayerEvent::Ready { slot, generation } => stage.ready(slot as usize, generation),
            PlayerEvent::VisualReady { slot, generation } => stage.visual_ready(slot as usize, generation),
            PlayerEvent::Stalled { slot, generation, position_ms } => stage.stalled(slot as usize, generation, position_ms),
            PlayerEvent::Failed { slot, generation, status, detail } => stage.failed(slot as usize, generation, status, detail),
            PlayerEvent::Ended { slot, generation } => stage.ended(slot as usize, generation),
        }
    }

    pub fn release(&mut self, owner: u64, kind: OwnerKind) {
        match kind {
            OwnerKind::Place => self.release_place(owner),
            OwnerKind::Window => self.release_window(owner),
            OwnerKind::Subscription => drop(self.subscriptions.remove(&owner)),
            OwnerKind::Declaration => {
                self.declarations.remove(&owner);
                self.assembled.stage.release_wholes_of(owner);
            }
            OwnerKind::Call => drop(self.calls.remove(&owner)),
            OwnerKind::Suspension => {
                self.assembled.stage.resume(owner);
                self.suspensions.remove(&owner);
            }
        }
    }

    pub fn terminate(&mut self, error: &MediaError) {
        for (owner, held) in self.subscriptions.drain() {
            if self.queue.is_live(owner) {
                held.listener.failed(error.clone());
            }
        }
    }

    pub(super) fn release_all(&mut self) {
        self.assembled.stage.release_all();
        for keyed in self.keys.values_mut() {
            keyed.applied = Applied::default();
        }
        self.video_applied = None;
    }

    // Places: incremental per-key aggregation

    pub(super) fn open_place(&mut self, owner: u64, item: MediaItem, reserve: Reserve) {
        let keyed = self.keys.entry(item.url.clone()).or_default();
        keyed.owners += 1;
        self.places.insert(owner, Place { item, input: PlaceInput::default(), declared: 0, intent: 0, _opened: reserve, updated: Reserve::none() });
    }

    pub(super) fn update_place(&mut self, owner: u64, input: PlaceInput, reserve: Reserve) {
        let intent = self.next_seq();
        let Some(place) = self.places.get_mut(&owner) else { return };
        let key = place.item.url.clone();
        let keyed = self.keys.entry(key.clone()).or_default();
        let previous = std::mem::replace(&mut place.input, input.clone());
        place.updated = reserve;
        withdraw(keyed, owner, &previous, place.declared, place.intent);
        place.intent = intent;
        place.declared = if input.visible { intent } else { 0 };
        contribute(keyed, owner, &input, place.declared, intent);
        self.flush(&key);
    }

    /// The first hold of a call stops all playback in one step and forgets every play and peek opinion: nothing plays again until an owner declares after the last release.
    fn hold_call(&mut self, owner: u64, reserve: Reserve) {
        let first = self.calls.is_empty();
        self.calls.insert(owner, reserve);
        if !first {
            return;
        }
        self.assembled.stage.stop_all_playing();
        for (owner, place) in self.places.iter_mut().filter(|(_, place)| place.input.playing || place.input.peeking) {
            let Some(keyed) = self.keys.get_mut(&place.item.url) else { continue };
            withdraw(keyed, *owner, &place.input, place.declared, place.intent);
            place.input.playing = false;
            place.input.peeking = false;
            contribute(keyed, *owner, &place.input, place.declared, place.intent);
        }
        let keys: Vec<String> = self.keys.iter().filter(|(_, keyed)| keyed.applied.playing || keyed.applied.peeking).map(|(key, _)| key.clone()).collect();
        for key in keys {
            self.flush(&key);
        }
    }

    pub(super) fn release_place(&mut self, owner: u64) {
        let Some(place) = self.places.remove(&owner) else { return };
        let key = place.item.url.clone();
        if let Some(keyed) = self.keys.get_mut(&key) {
            withdraw(keyed, owner, &place.input, place.declared, place.intent);
            keyed.owners -= 1;
        }
        self.flush(&key);
        if self.keys.get(&key).is_some_and(|keyed| keyed.owners == 0) {
            self.keys.remove(&key);
            if key.trim().is_empty() {
                self.assembled.stage.clear_refusal(&key);
            }
        }
    }

    pub(super) fn flush(&mut self, key: &str) {
        let stage = &self.assembled.stage;
        let Some(keyed) = self.keys.get_mut(key) else { return };
        if key.trim().is_empty() {
            stage.refuse_source(key);
            return;
        }
        let visible = keyed.visible.iter().next_back().and_then(|(_, owner)| self.places.get(owner)).map(|place| place.item.clone());
        match (visible, keyed.applied.source.is_some()) {
            (Some(source), false) => {
                stage.visible(source.clone(), true);
                keyed.applied.source = Some(source);
            }
            (Some(source), true) => {
                if keyed.applied.source.as_ref() != Some(&source) {
                    stage.replace_source(source.clone());
                    keyed.applied.source = Some(source);
                }
            }
            (None, _) => {}
        }
        let playing = keyed.playing > 0 && self.calls.is_empty();
        let groups: Vec<Option<String>> = keyed.groups.keys().cloned().collect();
        let token = keyed.token_owner().and_then(|owner| self.places.get(&owner)).and_then(|place| place.input.view_token);
        if playing != keyed.applied.playing || (playing && groups != keyed.applied.groups) {
            stage.set_playing(key, playing, &groups);
            keyed.applied.playing = playing;
            keyed.applied.groups = groups;
        }
        if token != keyed.applied.token {
            match token {
                Some(token) => stage.attach(key, token),
                None => stage.detach(key),
            }
            keyed.applied.token = token;
        }
        let peeking = keyed.peeking > 0 && self.calls.is_empty();
        if peeking != keyed.applied.peeking {
            stage.set_peeking(key, peeking);
            keyed.applied.peeking = peeking;
        }
        if let Some(muted) = keyed.muted() {
            if keyed.applied.muted != Some(muted) {
                stage.set_muted(key, muted);
                keyed.applied.muted = Some(muted);
            }
        }
        let rate = keyed.rate_owner().and_then(|owner| self.places.get(&owner)).and_then(|place| place.input.rate);
        match rate {
            Some(rate) if keyed.applied.rate != Some(rate) => {
                stage.set_rate(key, rate);
                keyed.applied.rate = Some(rate);
            }
            None if keyed.applied.rate.is_some() => {
                stage.set_rate(key, 1.0);
                keyed.applied.rate = None;
            }
            _ => {}
        }
        let pinned = keyed.is_pinned();
        if pinned != keyed.applied.pinned {
            if pinned { stage.pin(key) } else { stage.unpin(key) }
            keyed.applied.pinned = pinned;
        }
        if keyed.visible.is_empty() {
            if let Some(source) = keyed.applied.source.take() {
                stage.visible(source, false);
            }
        }
        if keyed.owners == 0 {
            keyed.applied = Applied::default();
        }
    }
}
