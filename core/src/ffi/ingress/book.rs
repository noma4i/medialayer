use super::command::{Answer, CoreAudio, Declared, Finished, OwnerKind, PlaceInput, PlayerEvent, Query};
use super::queue::{InputQueue, Reserve, EntryPlace};
use crate::api::{placeholder_for, AbortOnDrop, Hub, MediaError, MediaItem, MediaState, PlaybackState, PlayRefusal};
use crate::audio::PauseReason;
use tokio::runtime::Handle;
use super::super::assembled::Assembled;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use super::book_keys::*;
pub(crate) use super::book_types::Book;
use super::book_types::*;
use super::audio_rule::{self, AudioGlobal, PlaceAudio};
use super::AudioOp;

impl Book {
    pub fn new(assembled: Arc<Assembled>, queue: Arc<InputQueue>, handle: Handle) -> Book {
        assembled.stage.observe_playback();
        Book { assembled, queue, handle, seq: 0, places: HashMap::new(), keys: HashMap::new(), windows: HashMap::new(), known: HashMap::new(), video_order: BTreeMap::new(), video_applied: None, subscriptions: HashMap::new(), suspensions: HashMap::new(), suspension_groups: HashMap::new(), allowed_groups: HashMap::new(), in_call: false, paused: false, pause_reason: None, declarations: HashMap::new(), deliveries: tokio::task::JoinSet::new() }
    }

    pub(super) fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    pub fn apply(&mut self, command: Declared, reserve: Reserve, slot: EntryPlace) {
        let (command, wholes) = super::clean::declared(command, &mut self.known);
        match command {
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
            Declared::VideoWindow { owner, active, sources } => self.video_window(owner, active, sources, wholes, reserve),
            Declared::ImageWindow { owner, active, pages } => self.image_window(owner, active, pages, wholes, reserve),
            Declared::Suspend { owner, groups } => {
                self.assembled.stage.suspend(owner, &groups);
                let groups: Vec<String> = groups.into_iter().map(|group| group.trim().to_string()).collect();
                for group in groups.iter().collect::<std::collections::HashSet<_>>() { *self.allowed_groups.entry(group.clone()).or_default() += 1; }
                self.suspension_groups.insert(owner, groups);
                self.suspensions.insert(owner, reserve);
                self.publish_states();
            }
            Declared::Audio(action) => self.audio(action),
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
    pub(super) fn answer(&mut self, query: Query, reserve: Reserve, slot: EntryPlace) -> Answer {
        let assembled = self.assembled.clone();
        match query {
            Query::Retry(item) => {
                if assembled.places.retry(&item.url) {
                    assembled.stage.retry(&item);
                    if let Some(keyed) = self.keys.get_mut(&item.url) { keyed.applied.playing = false; }
                    self.flush(&item.url);
                    self.publish_key(&item.url);
                }
                Answer::Done
            }
            Query::ReadRoute(item) => Answer::Route(assembled.routes.play_url(&item)),
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
            OwnerKind::Subscription => {
                if let Some(observed) = self.subscriptions.remove(&owner) { self.finish_delivery(observed, None); }
            }
            OwnerKind::Declaration => {
                self.declarations.remove(&owner);
                self.assembled.stage.release_wholes_of(owner);
            }
            OwnerKind::Suspension => {
                self.assembled.stage.resume(owner);
                self.suspensions.remove(&owner);
                if let Some(groups) = self.suspension_groups.remove(&owner) {
                    for group in groups.iter().collect::<std::collections::HashSet<_>>() {
                        if let Some(count) = self.allowed_groups.get_mut(group) { *count -= 1; if *count == 0 { self.allowed_groups.remove(group); } }
                    }
                }
                self.publish_states();
            }
        }
    }

    pub fn terminate(&mut self, error: &MediaError) {
        for (owner, held) in std::mem::take(&mut self.subscriptions) {
            let error = self.queue.is_live(owner).then(|| error.clone());
            self.finish_delivery(held, error);
        }
    }

    pub(super) fn finish_delivery(&mut self, observed: Observed, error: Option<MediaError>) {
        use std::sync::atomic::Ordering;
        observed.delivery.closed.store(true, Ordering::Release);
        while self.deliveries.try_join_next().is_some() {}
        self.deliveries.spawn_blocking(move || {
            let _running = observed.delivery.running.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(error) = error { observed.listener.failed(error); }
        });
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
        keyed.owner_ids.insert(owner);
        let state = PlaybackState { playing: false, wish: false, reason: None, refused: None, attempt: 0 };
        self.places.insert(owner, Place { item, input: PlaceInput::default(), declared: 0, intent: 0, _opened: reserve, updated: Reserve::none(), emitted: state.clone(), state, forgotten: None, playback: Hub::new(self.handle.clone()), watchers: Vec::new() });
    }

    pub(super) fn update_place(&mut self, owner: u64, mut input: PlaceInput, reserve: Reserve) {
        let starts = self.places.get(&owner).is_some_and(|place| input.playing && (!place.input.playing || input.play_generation != place.input.play_generation) && place.forgotten != Some(input.play_generation));
        if let Some(reason) = self.pause_reason.filter(|reason| starts && !self.in_call && matches!(reason, PauseReason::Capture | PauseReason::Interrupted)) {
            crate::trace!("heal", "audio cleared pause={reason:?}");
            self.audio(CoreAudio::Op(AudioOp::ForgetPaused));
        }
        let intent = self.next_seq();
        let Some(place) = self.places.get_mut(&owner) else { return };
        let fresh = input.playing && (!place.input.playing || input.play_generation != place.input.play_generation) && place.forgotten != Some(input.play_generation);
        if input.playing && place.forgotten == Some(input.play_generation) { input.playing = false; }
        if fresh {
            let refusal = if self.in_call { Some(PlayRefusal::InCall) } else if self.pause_reason == Some(PauseReason::Capture) { Some(PlayRefusal::Recording) } else { None };
            place.state.reason = None;
            place.state.refused = refusal.clone();
            if refusal.is_some() { place.forgotten = Some(input.play_generation); input.playing = false; } else { place.forgotten = None; }
        } else if !input.playing && place.forgotten.is_none() {
            place.state.reason = None;
            place.state.refused = None;
        }
        let key = place.item.url.clone();
        let keyed = self.keys.entry(key.clone()).or_default();
        let was_playing = keyed.applied.playing;
        let previous = std::mem::replace(&mut place.input, input.clone());
        place.updated = reserve;
        withdraw(keyed, owner, &previous, place.declared, place.intent);
        place.intent = intent;
        place.declared = if input.visible { intent } else { 0 };
        contribute(keyed, owner, &input, place.declared, intent);
        self.flush(&key);
        if fresh && input.playing && was_playing && !self.in_call && !self.paused {
            let groups = self.keys[&key].groups.keys().cloned().collect::<Vec<_>>();
            self.assembled.stage.renew_playing(&key, &groups);
        }
        self.publish_key(&key);
    }

    fn audio(&mut self, action: CoreAudio) {
        let action = match action {
            CoreAudio::Op(action) => action,
            CoreAudio::Barrier(actions, sequence, progress) => {
                self.assembled.stage.begin_pause_barrier(sequence, progress);
                for action in actions { self.audio(CoreAudio::Op(action)); }
                self.assembled.stage.end_pause_barrier();
                return;
            }
        };
        let mut global = AudioGlobal { paused: self.paused, pause_reason: self.pause_reason, in_call: self.in_call };
        let owners: Vec<u64> = self.places.keys().copied().collect();
        let mut places: Vec<PlaceAudio> = owners.iter().map(|owner| {
            let place = &self.places[owner];
            PlaceAudio { playing: place.input.playing, peeking: place.input.peeking, forgotten: place.forgotten.is_some(), reason: place.state.reason }
        }).collect();
        let effects = audio_rule::step(&mut global, &mut places, &action);
        self.paused = global.paused;
        self.pause_reason = global.pause_reason;
        self.in_call = global.in_call;
        if effects.stop_all_playing { self.assembled.stage.stop_all_playing(); }
        for (owner, audio) in owners.into_iter().zip(places) {
            let Some(place) = self.places.get_mut(&owner) else { continue };
            place.state.reason = audio.reason;
            if effects.forget && (place.input.playing || place.input.peeking) && !audio.playing && !audio.peeking {
                let Some(keyed) = self.keys.get_mut(&place.item.url) else { continue };
                withdraw(keyed, owner, &place.input, place.declared, place.intent);
                place.input.playing = audio.playing;
                place.input.peeking = audio.peeking;
                place.forgotten = Some(place.input.play_generation);
                contribute(keyed, owner, &place.input, place.declared, place.intent);
            }
        }
        if effects.flush { self.flush_keys(); }
        if effects.reset_players { self.reset_players(); }
        if effects.publish { self.publish_states(); }
    }

    fn publish_states(&mut self) {
        let owners: Vec<u64> = self.places.keys().copied().collect();
        self.publish_owners(owners);
    }

    pub(super) fn publish_key(&mut self, key: &str) {
        let owners = self.keys.get(key).map(|keyed| keyed.owner_ids.iter().copied().collect()).unwrap_or_default();
        self.publish_owners(owners);
    }

    fn publish_owners(&mut self, owners: Vec<u64>) {
        for owner in owners {
            let Some(place) = self.places.get_mut(&owner) else { continue };
            let allows = |group: &str| self.allowed_groups.get(group).copied().unwrap_or(0) == self.suspensions.len();
            let suspended = !self.suspensions.is_empty() && !place.input.group.as_deref().map_or_else(|| crate::session::Context::Ring.policy().groups.iter().any(|group| allows(group)), |group| allows(group.trim()));
            let mut state = place.state.clone();
            state.wish = place.input.playing;
            state.attempt = self.assembled.places.attempt(&place.item.url);
            state.playing = state.wish && !self.paused && !self.in_call && !suspended && self.assembled.stage.is_playing(&place.item.url);
            if state.wish && suspended && !self.paused && !self.in_call { state.reason = Some(PauseReason::Suspended); }
            else if state.reason == Some(PauseReason::Suspended) && !suspended { state.reason = None; }
            else if state.wish && state.playing { state.reason = None; }
            place.state = state.clone();
            if state != place.emitted { place.emitted = state.clone(); place.playback.publish(state); }
        }
    }

    fn flush_keys(&mut self) {
        let keys: Vec<String> = self.keys.keys().cloned().collect();
        for key in keys {
            self.flush(&key);
        }
    }

    fn reset_players(&mut self) {
        self.assembled.stage.reset_players();
        for keyed in self.keys.values_mut() {
            keyed.applied.muted = None;
            keyed.applied.pinned = false;
        }
        self.video_applied = None;
        self.resolve_video();
        self.flush_keys();
    }

    pub(super) fn release_place(&mut self, owner: u64) {
        let Some(place) = self.places.remove(&owner) else { return };
        for watcher in &place.watchers {
            if let Some(observed) = self.subscriptions.remove(watcher) {
                let error = self.queue.is_live(*watcher).then_some(MediaError::Closed);
                self.finish_delivery(observed, error);
            }
        }
        let key = place.item.url.clone();
        if let Some(keyed) = self.keys.get_mut(&key) {
            withdraw(keyed, owner, &place.input, place.declared, place.intent);
            keyed.owners -= 1;
            keyed.owner_ids.remove(&owner);
        }
        self.flush(&key);
        self.publish_key(&key);
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
        let playing = keyed.playing > 0 && !self.in_call && !self.paused;
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
        let peeking = keyed.peeking > 0 && !self.in_call && !self.paused;
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
