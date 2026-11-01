use super::super::assembled::Assembled;
use super::super::layer::WindowReadyListener;
use crate::api::{AbortOnDrop, Kind, MediaItem, MediaState};
use crate::prefetch::{CacheStrategy, Outcome, Settled};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::runtime::Handle;
use tokio::sync::{broadcast, watch};

#[derive(Default)]
struct Shown {
    epoch: u64,
    ready: bool,
    listener: Option<Arc<dyn WindowReadyListener>>,
}

#[derive(Default)]
struct Sink(Mutex<Shown>);

impl Sink {
    fn shown(&self) -> MutexGuard<'_, Shown> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn begin(&self) -> u64 {
        let mut shown = self.shown();
        shown.epoch += 1;
        shown.epoch
    }

    fn pending(&self, epoch: u64) {
        let mut shown = self.shown();
        if shown.epoch != epoch || !std::mem::take(&mut shown.ready) {
            return;
        }
        if let Some(listener) = &shown.listener {
            listener.changed(false);
        }
    }

    fn ready(&self, epoch: u64) {
        let mut shown = self.shown();
        if shown.epoch != epoch || shown.ready {
            return;
        }
        shown.ready = true;
        if let Some(listener) = &shown.listener {
            listener.changed(true);
        }
    }

    fn listen(&self, listener: Arc<dyn WindowReadyListener>) {
        let mut shown = self.shown();
        if shown.ready {
            listener.changed(true);
        }
        shown.listener = Some(listener);
    }

    fn listening(&self) -> bool {
        self.shown().listener.is_some()
    }

    fn close(&self) {
        let mut shown = self.shown();
        shown.epoch += 1;
        shown.listener = None;
    }
}

pub(super) struct PageReadiness {
    sink: Arc<Sink>,
    watched: Option<Option<Vec<MediaItem>>>,
    _task: Option<AbortOnDrop>,
}

impl PageReadiness {
    pub fn new() -> PageReadiness {
        PageReadiness { sink: Arc::default(), watched: None, _task: None }
    }

    pub fn listen(&mut self, listener: Arc<dyn WindowReadyListener>) {
        self.sink.listen(listener);
    }

    pub fn follow(&mut self, assembled: &Arc<Assembled>, handle: &Handle, page: Option<Vec<MediaItem>>) {
        if !self.sink.listening() || self.watched.as_ref() == Some(&page) {
            return;
        }
        let epoch = self.sink.begin();
        if page.is_none() {
            self.sink.pending(epoch);
        }
        self._task = page.clone().map(|items| {
            // Every receiver exists before the task runs: an outcome that lands while the cache is checked or an earlier source is awaited stays queued for its own.
            let receivers = items.iter().map(|_| assembled.stage.settled()).collect();
            AbortOnDrop::new(handle.spawn(prepare(assembled.clone(), items, receivers, self.sink.clone(), epoch)))
        });
        self.watched = Some(page);
    }
}

impl Drop for PageReadiness {
    fn drop(&mut self) {
        self.sink.close();
    }
}

async fn prepare(assembled: Arc<Assembled>, items: Vec<MediaItem>, receivers: Vec<broadcast::Receiver<Settled>>, sink: Arc<Sink>, epoch: u64) {
    // A page whose sources are already shown or cached keeps the window ready: the report never dips to false between two ready pages.
    if all_available(&assembled, &items).await {
        sink.ready(epoch);
        return;
    }
    sink.pending(epoch);
    for (item, settled) in items.iter().zip(receivers) {
        if item.kind == Kind::Video {
            first_frame_or_cached(&assembled, item, settled).await;
        } else {
            cached_or_failed(&assembled, item, settled).await;
        }
    }
    sink.ready(epoch);
}

async fn first_frame_or_cached(assembled: &Assembled, item: &MediaItem, mut settled: broadcast::Receiver<Settled>) {
    let (state, mut rendered) = watch::channel(MediaState::Idle);
    let _subscription = assembled.places.subscribe(&item.url, Arc::new(move |next| {
        state.send_replace(next);
    }));
    tokio::select! {
        _ = async { rendered.wait_for(|state| matches!(state, MediaState::Rendered | MediaState::Ended)).await.is_ok() } => {}
        _ = cached(assembled, item, &mut settled) => {}
    }
}

async fn all_available(assembled: &Assembled, items: &[MediaItem]) -> bool {
    for item in items {
        let local = item.kind != Kind::Video && (item.is_local() || item.strategy == CacheStrategy::OnDemand);
        if !local && !assembled.routes.is_cached(&item.url, &item.headers).await {
            return false;
        }
    }
    true
}

/// Ends when the source is in the cache: at once, or when a load of that url ends.
async fn cached(assembled: &Assembled, item: &MediaItem, settled: &mut broadcast::Receiver<Settled>) {
    loop {
        if assembled.routes.is_cached(&item.url, &item.headers).await {
            return;
        }
        loop {
            match settled.recv().await {
                Ok(done) if done.url == item.url && done.outcome == Outcome::Loaded => return,
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => break,
                Err(broadcast::error::RecvError::Closed) => std::future::pending::<()>().await,
            }
        }
    }
}

async fn cached_or_failed(assembled: &Assembled, item: &MediaItem, mut settled: broadcast::Receiver<Settled>) {
    if item.is_local() || item.strategy == CacheStrategy::OnDemand {
        return;
    }
    loop {
        if assembled.routes.is_cached(&item.url, &item.headers).await {
            return;
        }
        loop {
            match settled.recv().await {
                Ok(done) if done.url == item.url && done.outcome != Outcome::Cancelled => return,
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => break,
                Err(broadcast::error::RecvError::Closed) => std::future::pending::<()>().await,
            }
        }
    }
}
