use super::command::{Observer, Subscribing, WindowKind};
use super::queue::Reserve;
use super::readiness::PageReadiness;
use super::super::layer::WindowReadyListener;
use crate::api::MediaItem;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use super::book_types::*;

impl Book {
    // Windows

    pub(super) fn open_window(&mut self, owner: u64, kind: WindowKind, reserve: Reserve) {
        let window = match kind {
            WindowKind::Video => Window::Video(VideoWindowState { sources: Arc::new(Vec::new()), active: 0, is_active: true, intent: 0, replaced: 0, ready: PageReadiness::new(), _opened: reserve, held: Reserve::none() }),
            WindowKind::Image(name) => Window::Image(ImageWindowState { scoped: format!("{name}\u{1f}{owner}"), pages: Arc::new(Vec::new()), active: 0, is_active: true, ready: PageReadiness::new(), _opened: reserve, held: Reserve::none() }),
        };
        self.windows.insert(owner, window);
    }

    pub(super) fn set_window_active(&mut self, owner: u64, is_active: bool) {
        let intent = self.next_seq();
        match self.windows.get_mut(&owner) {
            Some(Window::Video(window)) => {
                window.is_active = is_active;
                self.video_order.remove(&window.intent);
                window.intent = intent;
                if is_active && (self.video_applied.is_some_and(|(applied, _)| applied == owner) || window.declares_source()) {
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

    pub(super) fn video_window(&mut self, owner: u64, active: u32, sources: Option<Arc<Vec<Option<MediaItem>>>>, reserve: Reserve) {
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
        if window.is_active && (self.video_applied.is_some_and(|(applied, _)| applied == owner) || window.declares_source()) {
            self.video_order.insert(intent, owner);
        }
        self.follow_ready(owner);
        self.resolve_video();
    }

    pub(super) fn window_ready(&mut self, owner: u64, listener: Arc<dyn WindowReadyListener>) {
        match self.windows.get_mut(&owner) {
            Some(Window::Video(window)) => window.ready.listen(listener),
            Some(Window::Image(window)) => window.ready.listen(listener),
            None => return,
        }
        self.follow_ready(owner);
    }

    pub(super) fn follow_ready(&mut self, owner: u64) {
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

    pub(super) fn resolve_video(&mut self) {
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

    pub(super) fn image_window(&mut self, owner: u64, active: u32, pages: Option<Arc<Vec<Vec<MediaItem>>>>, reserve: Reserve) {
        let Some(Window::Image(window)) = self.windows.get_mut(&owner) else { return };
        window.active = active;
        let replaced = pages.is_some();
        if let Some(pages) = pages {
            drop(std::mem::replace(&mut window.pages, pages));
            window.held = reserve;
        }
        self.follow_ready(owner);
        if replaced { self.apply_image(owner) } else { self.move_image(owner, active) }
    }

    pub(super) fn move_image(&self, owner: u64, active: u32) {
        let Some(Window::Image(window)) = self.windows.get(&owner) else { return };
        if window.is_active {
            self.assembled.stage.move_image_window(&window.scoped, active as usize);
        }
    }

    pub(super) fn apply_image(&self, owner: u64) {
        let Some(Window::Image(window)) = self.windows.get(&owner) else { return };
        let pages = if window.is_active { window.pages.as_ref().clone() } else { Vec::new() };
        self.assembled.stage.replace_image_window(&window.scoped, window.active as usize, pages);
    }

    pub(super) fn release_window(&mut self, owner: u64) {
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

    pub(super) fn subscribe(&mut self, owner: u64, subscribing: Subscribing, reserve: Reserve) {
        let gate = self.queue.clone();
        let delivery = Arc::new(Delivery { closed: AtomicBool::new(false), running: Mutex::new(()) });
        let serial = delivery.clone();
        let (subscription, listener) = match subscribing {
            Subscribing::Playback { place, listener } => {
                let Some(place) = self.places.get_mut(&place) else { return listener.failed(crate::api::MediaError::Closed) };
                place.watchers.push(owner);
                let target = listener.clone();
                let serial = serial.clone();
                let gate = gate.clone();
                let subscription = place.playback.subscribe(Arc::new(move |state| serial.send(&gate, owner, || target.changed(state))));
                place.playback.publish(place.state.clone());
                (subscription, Observer::Playback(listener))
            }
            Subscribing::State { key, listener } => {
                let target = listener.clone();
                let serial = serial.clone();
                let gate = gate.clone();
                let subscription = self.assembled.places.subscribe(&key, Arc::new(move |state| serial.send(&gate, owner, || target.changed(state))));
                (subscription, Observer::State(listener))
            }
            Subscribing::Stats { listener } => {
                let target = listener.clone();
                let serial = serial.clone();
                let gate = gate.clone();
                let subscription = self.assembled.controls.subscribe_stats(Arc::new(move |stats| serial.send(&gate, owner, || target.changed(stats))));
                (subscription, Observer::Stats(listener))
            }
            Subscribing::Tone { item, band, listener } => {
                let target = listener.clone();
                let serial = serial.clone();
                let gate = gate.clone();
                let subscribed = self.assembled.tones.subscribe(&item, band, Arc::new(move |tone| serial.send(&gate, owner, || target.changed(tone))));
                match subscribed {
                    Ok(subscription) => (subscription, Observer::Tone(listener)),
                    Err(error) => return listener.failed(error.into()),
                }
            }
        };
        self.subscriptions.insert(owner, Observed { _subscription: subscription, listener, _reserve: reserve, delivery });
    }
}
