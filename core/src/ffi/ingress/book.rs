use super::command::{Answer, Declared, Observer, Finished, OwnerKind, PlaceInput, PlayerEvent, Query, Subscribing, WindowKind};
use super::queue::{InputQueue, Reserve, EntryPlace};
use super::readiness::PageReadiness;
use super::super::layer::WindowReadyListener;
use crate::api::{placeholder_for, AbortOnDrop, CacheState, Kind, MediaError, MediaItem, MediaState, Subscription};
use tokio::runtime::Handle;
use super::super::assembled::Assembled;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

#[derive(Default)]
struct Applied {
    source: Option<MediaItem>,
    playing: bool,
    groups: Vec<Option<String>>,
    peeking: bool,
    muted: Option<bool>,
    pinned: bool,
    rate: Option<f32>,
    token: Option<u64>,
}

#[derive(Default)]
struct KeyOwners {
    owners: usize,
    visible: BTreeMap<u64, u64>,
    playing: usize,
    peeking: usize,
    muted_some: usize,
    muted_true: usize,
    pins: usize,
    groups: BTreeMap<Option<String>, usize>,
    tokens: BTreeSet<(u8, u64, u64)>,
    rates: BTreeSet<(u8, u64, u64)>,
    applied: Applied,
}

impl KeyOwners {
    /// `None` while no owner has an opinion; otherwise muted only when every opinion asks for mute.
    fn muted(&self) -> Option<bool> {
        (self.muted_some > 0).then(|| self.muted_true == self.muted_some)
    }

    fn is_pinned(&self) -> bool {
        self.pins > 0
    }

    /// The owner whose view and rate win: playing first, then visible, then the newest.
    fn token_owner(&self) -> Option<u64> {
        self.tokens.iter().next_back().map(|(_, _, owner)| *owner)
    }

    fn rate_owner(&self) -> Option<u64> {
        self.rates.iter().next_back().map(|(_, _, owner)| *owner)
    }
}

struct Place {
    item: MediaItem,
    input: PlaceInput,
    declared: u64,
    intent: u64,
    _opened: Reserve,
    updated: Reserve,
}

struct VideoWindow {
    sources: Arc<Vec<Option<MediaItem>>>,
    active: u32,
    is_active: bool,
    intent: u64,
    replaced: u64,
    ready: PageReadiness,
    _opened: Reserve,
    held: Reserve,
}

struct ImageWindow {
    scoped: String,
    pages: Arc<Vec<Vec<MediaItem>>>,
    active: u32,
    is_active: bool,
    ready: PageReadiness,
    _opened: Reserve,
    held: Reserve,
}

enum Window {
    Video(VideoWindow),
    Image(ImageWindow),
}

struct Observed {
    _subscription: Subscription,
    listener: Observer,
    _reserve: Reserve,
}

/// Executor-owned state: aggregation per key, windows, subscriptions. Single-threaded, no locks.
pub(crate) struct Book {
    assembled: Arc<Assembled>,
    queue: Arc<InputQueue>,
    handle: Handle,
    seq: u64,
    places: HashMap<u64, Place>,
    keys: HashMap<String, KeyOwners>,
    windows: HashMap<u64, Window>,
    video_order: BTreeMap<u64, u64>,
    video_applied: Option<(u64, u64)>,
    subscriptions: HashMap<u64, Observed>,
    suspensions: HashMap<u64, Reserve>,
    declarations: HashMap<u64, Reserve>,
}

fn rank(input: &PlaceInput) -> u8 {
    u8::from(input.playing) * 2 + u8::from(input.visible)
}

impl Book {
    pub fn new(assembled: Arc<Assembled>, queue: Arc<InputQueue>, handle: Handle) -> Book {
        Book { assembled, queue, handle, seq: 0, places: HashMap::new(), keys: HashMap::new(), windows: HashMap::new(), video_order: BTreeMap::new(), video_applied: None, subscriptions: HashMap::new(), suspensions: HashMap::new(), declarations: HashMap::new() }
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    pub fn apply(&mut self, command: Declared, reserve: Reserve, slot: EntryPlace) {
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
            Declared::VideoWindow { owner, active, sources } => self.video_window(owner, active, sources, reserve),
            Declared::ImageWindow { owner, active, pages } => self.image_window(owner, active, pages, reserve),
            Declared::Suspend { owner, groups } => {
                self.assembled.stage.suspend(owner, &groups);
                self.suspensions.insert(owner, reserve);
            }
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
    fn answer(&self, query: Query, reserve: Reserve, slot: EntryPlace) -> Answer {
        let assembled = self.assembled.clone();
        match query {
            Query::PlayUrl(item) => Answer::Route(assembled.routes.play_url(&item)),
            Query::Refresh(item, route) => Answer::Route(assembled.routes.refresh(&item, &route)),
            Query::Pool => Answer::Pool(assembled.pool_stats()),
            Query::Placeholder(item, ready) if item.kind != Kind::Video || ready => Answer::Placeholder(placeholder_for(item, CacheState::Missing, ready)),
            Query::Placeholder(item, ready) => self.running(reserve, slot, async move {
                let cache = if assembled.routes.is_cached(&item.url, &item.headers).await { CacheState::Cached } else { CacheState::Missing };
                Finished::Placeholder(placeholder_for(item, cache, ready))
            }),
            Query::Cached(item) => self.running(reserve, slot, async move { Finished::Cached(assembled.routes.is_cached(&item.url, &item.headers).await) }),
            Query::Tone(item, band) => self.running(reserve, slot, async move { Finished::Tone(assembled.tones.tone(&item, band).await) }),
            Query::Pixels(item) => self.running(reserve, slot, async move { Finished::Pixels(assembled.tones.pixels_needed(&item).await) }),
            Query::ImageSize { url, headers } => self.running(reserve, slot, async move { Finished::ImageSize(assembled.routes.image_size(&url, &headers).await) }),
            Query::Image { url, headers } => self.running(reserve, slot, async move {
                let result = assembled.routes.image_bytes(&url, &headers, crate::memory::InputLimits::default().response_bytes).await;
                assembled.stage.announce_image(&url, result.as_ref().map(|_| ()).map_err(ToString::to_string));
                Finished::Image(result)
            }),
        }
    }

    /// The reservation and the entry place move into the work and are freed when it ends or is dropped, never earlier.
    fn running(&self, reserve: Reserve, slot: EntryPlace, work: impl std::future::Future<Output = Finished> + Send + 'static) -> Answer {
        Answer::Running(AbortOnDrop::new(self.handle.spawn(async move {
            let _held = (reserve, slot);
            work.await
        })))
    }

    fn player(&self, event: PlayerEvent) {
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

    fn release_all(&mut self) {
        self.assembled.stage.release_all();
        for keyed in self.keys.values_mut() {
            keyed.applied = Applied::default();
        }
        self.video_applied = None;
    }

    // Places: incremental per-key aggregation

    fn open_place(&mut self, owner: u64, item: MediaItem, reserve: Reserve) {
        let keyed = self.keys.entry(item.url.clone()).or_default();
        keyed.owners += 1;
        self.places.insert(owner, Place { item, input: PlaceInput::default(), declared: 0, intent: 0, _opened: reserve, updated: Reserve::none() });
    }

    fn update_place(&mut self, owner: u64, input: PlaceInput, reserve: Reserve) {
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

    fn release_place(&mut self, owner: u64) {
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

    fn flush(&mut self, key: &str) {
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
        let playing = keyed.playing > 0;
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
        let peeking = keyed.peeking > 0;
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

    // Windows

    fn open_window(&mut self, owner: u64, kind: WindowKind, reserve: Reserve) {
        let window = match kind {
            WindowKind::Video => Window::Video(VideoWindow { sources: Arc::new(Vec::new()), active: 0, is_active: true, intent: 0, replaced: 0, ready: PageReadiness::new(), _opened: reserve, held: Reserve::none() }),
            WindowKind::Image(name) => Window::Image(ImageWindow { scoped: format!("{name}\u{1f}{owner}"), pages: Arc::new(Vec::new()), active: 0, is_active: true, ready: PageReadiness::new(), _opened: reserve, held: Reserve::none() }),
        };
        self.windows.insert(owner, window);
    }

    fn set_window_active(&mut self, owner: u64, is_active: bool) {
        let intent = self.next_seq();
        match self.windows.get_mut(&owner) {
            Some(Window::Video(window)) => {
                window.is_active = is_active;
                self.video_order.remove(&window.intent);
                window.intent = intent;
                if is_active {
                    self.video_order.insert(intent, owner);
                }
                self.resolve_video();
            }
            Some(Window::Image(window)) => {
                window.is_active = is_active;
                self.apply_image(owner);
            }
            None => {}
        }
    }

    fn video_window(&mut self, owner: u64, active: u32, sources: Option<Arc<Vec<Option<MediaItem>>>>, reserve: Reserve) {
        let intent = self.next_seq();
        let replaced = self.next_seq();
        let Some(Window::Video(window)) = self.windows.get_mut(&owner) else { return };
        window.active = active;
        if let Some(sources) = sources {
            drop(std::mem::replace(&mut window.sources, sources));
            window.held = reserve;
            window.replaced = replaced;
        }
        self.video_order.remove(&window.intent);
        window.intent = intent;
        if window.is_active {
            self.video_order.insert(intent, owner);
        }
        self.resolve_video();
        self.follow_ready(owner);
    }

    fn window_ready(&mut self, owner: u64, listener: Arc<dyn WindowReadyListener>) {
        match self.windows.get_mut(&owner) {
            Some(Window::Video(window)) => window.ready.listen(listener),
            Some(Window::Image(window)) => window.ready.listen(listener),
            None => return,
        }
        self.follow_ready(owner);
    }

    fn follow_ready(&mut self, owner: u64) {
        let (assembled, handle) = (&self.assembled, &self.handle);
        match self.windows.get_mut(&owner) {
            Some(Window::Video(window)) => {
                let page = window.sources.get(window.active as usize).cloned().flatten().map(|item| vec![item]);
                window.ready.follow(assembled, handle, page);
            }
            Some(Window::Image(window)) => {
                let page = window.pages.get(window.active as usize).cloned();
                window.ready.follow(assembled, handle, page);
            }
            None => {}
        }
    }

    fn resolve_video(&mut self) {
        let winner = self.video_order.iter().next_back().map(|(_, owner)| *owner);
        let stage = &self.assembled.stage;
        match winner.and_then(|owner| match self.windows.get(&owner) {
            Some(Window::Video(window)) => Some((owner, window)),
            _ => None,
        }) {
            Some((owner, window)) => {
                let applied = (owner, window.replaced);
                if self.video_applied != Some(applied) {
                    stage.replace_window(window.active as usize, window.sources.as_ref().clone());
                    self.video_applied = Some(applied);
                } else {
                    stage.move_window(window.active as usize);
                }
            }
            None => {
                if self.video_applied.take().is_some() {
                    stage.replace_window(0, Vec::new());
                }
            }
        }
    }

    fn image_window(&mut self, owner: u64, active: u32, pages: Option<Arc<Vec<Vec<MediaItem>>>>, reserve: Reserve) {
        let Some(Window::Image(window)) = self.windows.get_mut(&owner) else { return };
        window.active = active;
        let replaced = pages.is_some();
        if let Some(pages) = pages {
            drop(std::mem::replace(&mut window.pages, pages));
            window.held = reserve;
        }
        if replaced { self.apply_image(owner) } else if window.is_active { self.assembled.stage.move_image_window(&window.scoped, active as usize) }
        self.follow_ready(owner);
    }

    fn apply_image(&self, owner: u64) {
        let Some(Window::Image(window)) = self.windows.get(&owner) else { return };
        let pages = if window.is_active { window.pages.as_ref().clone() } else { Vec::new() };
        self.assembled.stage.replace_image_window(&window.scoped, window.active as usize, pages);
    }

    fn release_window(&mut self, owner: u64) {
        match self.windows.remove(&owner) {
            Some(Window::Video(window)) => {
                self.video_order.remove(&window.intent);
                self.resolve_video();
            }
            Some(Window::Image(window)) => self.assembled.stage.close_image_window(&window.scoped),
            None => {}
        }
    }

    // Subscriptions

    fn subscribe(&mut self, owner: u64, subscribing: Subscribing, reserve: Reserve) {
        let gate = self.queue.clone();
        let live = move || gate.is_live(owner);
        let (subscription, listener) = match subscribing {
            Subscribing::State { key, listener } => {
                let deliver = listener.clone();
                let subscription = self.assembled.places.subscribe(&key, Arc::new(move |state| if live() { deliver.changed(state) }));
                (subscription, Observer::State(listener))
            }
            Subscribing::Stats { listener } => {
                let deliver = listener.clone();
                let subscription = self.assembled.controls.subscribe_stats(Arc::new(move |stats| if live() { deliver.changed(stats) }));
                (subscription, Observer::Stats(listener))
            }
            Subscribing::Tone { item, band, listener } => {
                let deliver = listener.clone();
                let subscribed = self.assembled.tones.subscribe(&item, band, Arc::new(move |tone| if live() { deliver.changed(tone) }));
                match subscribed {
                    Ok(subscription) => (subscription, Observer::Tone(listener)),
                    Err(error) => return listener.failed(error.into()),
                }
            }
        };
        self.subscriptions.insert(owner, Observed { _subscription: subscription, listener, _reserve: reserve });
    }
}

fn withdraw(keyed: &mut KeyOwners, owner: u64, input: &PlaceInput, declared: u64, intent: u64) {
    if declared != 0 {
        keyed.visible.remove(&declared);
    }
    if input.playing {
        keyed.playing -= 1;
        if let Some(count) = keyed.groups.get_mut(&input.group) {
            *count -= 1;
            if *count == 0 {
                keyed.groups.remove(&input.group);
            }
        }
    }
    keyed.peeking -= usize::from(input.peeking);
    if let Some(muted) = input.muted {
        keyed.muted_some -= 1;
        keyed.muted_true -= usize::from(muted);
    }
    keyed.pins -= usize::from(input.pinned);
    if input.view_token.is_some() {
        keyed.tokens.remove(&(rank(input), intent, owner));
    }
    if input.rate.is_some() {
        keyed.rates.remove(&(rank(input), intent, owner));
    }
}

fn contribute(keyed: &mut KeyOwners, owner: u64, input: &PlaceInput, declared: u64, intent: u64) {
    if declared != 0 {
        keyed.visible.insert(declared, owner);
    }
    if input.playing {
        keyed.playing += 1;
        *keyed.groups.entry(input.group.clone()).or_default() += 1;
    }
    keyed.peeking += usize::from(input.peeking);
    if let Some(muted) = input.muted {
        keyed.muted_some += 1;
        keyed.muted_true += usize::from(muted);
    }
    keyed.pins += usize::from(input.pinned);
    if input.view_token.is_some() {
        keyed.tokens.insert((rank(input), intent, owner));
    }
    if input.rate.is_some() {
        keyed.rates.insert((rank(input), intent, owner));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(visible: bool, playing: bool, group: Option<&str>) -> PlaceInput {
        PlaceInput { visible, playing, group: group.map(str::to_string), ..PlaceInput::default() }
    }

    fn join(keyed: &mut KeyOwners, owner: u64, input: &PlaceInput, intent: u64) {
        contribute(keyed, owner, input, if input.visible { intent } else { 0 }, intent);
    }

    #[test]
    fn inv_ingress_22_visible_and_peeking_are_or_and_the_source_returns_to_the_last_remaining_declaration() {
        let mut keyed = KeyOwners::default();
        let (first, second) = (input(true, false, None), PlaceInput { visible: true, peeking: true, ..PlaceInput::default() });
        join(&mut keyed, 1, &first, 1);
        join(&mut keyed, 2, &second, 2);
        assert_eq!(keyed.visible.iter().next_back().map(|(_, owner)| *owner), Some(2));

        withdraw(&mut keyed, 2, &second, 2, 2);

        assert_eq!(keyed.visible.iter().next_back().map(|(_, owner)| *owner), Some(1));
        assert_eq!(keyed.peeking, 0);
        withdraw(&mut keyed, 1, &first, 1, 1);
        assert!(keyed.visible.is_empty());
    }

    #[test]
    fn inv_ingress_23_playing_is_or_and_groups_are_the_set_of_the_playing_owners() {
        let mut keyed = KeyOwners::default();
        let (chat, voice, idle) = (input(false, true, Some("chat")), input(false, true, Some("voice")), input(false, false, Some("idle")));
        join(&mut keyed, 1, &chat, 1);
        join(&mut keyed, 2, &voice, 2);
        join(&mut keyed, 3, &idle, 3);
        assert_eq!((keyed.playing, keyed.groups.keys().cloned().collect::<Vec<_>>()), (2, vec![Some("chat".to_string()), Some("voice".to_string())]));

        withdraw(&mut keyed, 2, &voice, 0, 2);

        assert_eq!((keyed.playing, keyed.groups.keys().cloned().collect::<Vec<_>>()), (1, vec![Some("chat".to_string())]));
        let ungrouped = input(false, true, None);
        join(&mut keyed, 4, &ungrouped, 4);
        assert_eq!(keyed.groups.keys().cloned().collect::<Vec<_>>(), vec![None, Some("chat".to_string())]);
    }

    #[test]
    fn inv_ingress_24_a_key_is_muted_only_while_every_opinion_asks_for_mute() {
        let mut keyed = KeyOwners::default();
        let asking = PlaceInput { muted: Some(true), ..PlaceInput::default() };
        let refusing = PlaceInput { muted: Some(false), ..PlaceInput::default() };
        join(&mut keyed, 1, &asking, 1);
        join(&mut keyed, 2, &refusing, 2);
        assert_eq!(keyed.muted(), Some(false));

        withdraw(&mut keyed, 2, &refusing, 0, 2);
        assert_eq!(keyed.muted(), Some(true));
        join(&mut keyed, 2, &asking, 3);

        assert_eq!(keyed.muted(), Some(true));
        withdraw(&mut keyed, 1, &asking, 0, 1);
        withdraw(&mut keyed, 2, &asking, 0, 3);
        assert_eq!(keyed.muted(), None);
    }

    #[test]
    fn inv_ingress_25_the_view_and_the_rate_come_from_the_best_live_owner_playing_first_then_visible_then_newest() {
        let mut keyed = KeyOwners::default();
        let shown = PlaceInput { visible: true, view_token: Some(1), rate: Some(1.5), ..PlaceInput::default() };
        let playing = PlaceInput { playing: true, view_token: Some(2), rate: Some(2.0), ..PlaceInput::default() };
        let newer = PlaceInput { view_token: Some(3), ..PlaceInput::default() };
        join(&mut keyed, 1, &shown, 1);
        join(&mut keyed, 2, &playing, 2);
        join(&mut keyed, 3, &newer, 3);
        assert_eq!(keyed.token_owner(), Some(2));
        assert_eq!(keyed.rate_owner(), Some(2));

        withdraw(&mut keyed, 2, &playing, 0, 2);

        assert_eq!(keyed.token_owner(), Some(1));
        assert_eq!(keyed.rate_owner(), Some(1));
    }

    #[test]
    fn inv_ingress_26_a_pin_lives_while_any_owner_pins() {
        let mut keyed = KeyOwners::default();
        let pinned = PlaceInput { pinned: true, ..PlaceInput::default() };
        join(&mut keyed, 1, &pinned, 1);
        join(&mut keyed, 2, &pinned, 2);

        withdraw(&mut keyed, 1, &pinned, 0, 1);
        assert!(keyed.is_pinned());
        withdraw(&mut keyed, 2, &pinned, 0, 2);

        assert!(!keyed.is_pinned());
    }
}
