use super::guard::AbortOnDrop;
use super::hub::{Deliver, Hub, Subscription};
use super::ports::Suspendable;
use super::source::MediaItem;
use crate::analysis::{analyze, analyze_blurhash, valid_band, MediaTone, ToneData, ToneError, ToneSource};
use crate::cache::Cache;
use crate::proxy::block_key;
use async_trait::async_trait;
use futures_util::future::{AbortHandle, Abortable};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use tokio::runtime::Handle;
use tokio::sync::Notify;

pub const MEMO_ENTRIES: usize = 256;

#[derive(uniffi::Record)]
pub struct TonePixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[uniffi::export(with_foreign)]
#[async_trait]
pub trait TonePreparation: Send + Sync {
    async fn prepare(&self, source: MediaItem) -> Option<TonePixels>;
}

struct Demand {
    source: MediaItem,
    queried: bool,
    seq: u64,
}

#[async_trait]
pub trait ToneStore: Send + Sync {
    fn key_for(&self, url: &str, headers: &HashMap<String, String>) -> String;
    async fn tone(&self, key: &str) -> Option<ToneData>;
    async fn set_tone(&self, key: &str, tone: ToneData) -> bool;
}

pub type Keying = Arc<dyn Fn(&str, &HashMap<String, String>) -> String + Send + Sync>;

pub struct CacheTones {
    cache: Cache,
    keying: Keying,
}

impl CacheTones {
    pub fn new(cache: Cache, keying: Keying) -> CacheTones {
        CacheTones { cache, keying }
    }
}

#[async_trait]
impl ToneStore for CacheTones {
    fn key_for(&self, url: &str, headers: &HashMap<String, String>) -> String {
        (self.keying)(url, headers)
    }

    async fn tone(&self, key: &str) -> Option<ToneData> {
        self.cache.tone(key).or_else(|| self.cache.tone(&block_key(key, 0)))
    }

    async fn set_tone(&self, key: &str, tone: ToneData) -> bool {
        self.cache.set_tone(key, tone.clone()) || self.cache.set_tone(&block_key(key, 0), tone)
    }
}

struct Memo {
    capacity: usize,
    order: VecDeque<String>,
    tones: HashMap<String, ToneData>,
}

impl Memo {
    fn insert(&mut self, key: String, tone: ToneData) {
        if self.tones.insert(key.clone(), tone).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.tones.remove(&oldest);
            }
        }
    }

    fn remove(&mut self, key: &str) {
        if self.tones.remove(key).is_some() {
            self.order.retain(|queued| queued != key);
        }
    }
}

type Hubs = Arc<Mutex<HashMap<String, Hub<ToneSource>>>>;

pub struct Tones {
    handle: Handle,
    store: Arc<dyn ToneStore>,
    memo: Mutex<Memo>,
    running: Mutex<HashMap<String, AbortHandle>>,
    demands: Mutex<HashMap<String, Demand>>,
    preparation: Mutex<Option<Arc<dyn TonePreparation>>>,
    hubs: Hubs,
    analyses: AtomicU64,
    suspended: AtomicBool,
    closed: AtomicBool,
    active: AtomicUsize,
    idle: Notify,
    demand_seq: AtomicU64,
    #[cfg(test)]
    delivered_changes: Arc<AtomicU64>,
}

struct Running {
    tones: Arc<Tones>,
    key: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        locked(&self.tones.running).remove(&self.key);
        if self.tones.active.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.tones.idle.notify_waiters();
        }
        #[cfg(test)]
        crate::test_signal::fire();
    }
}

fn locked<V>(mutex: &Mutex<V>) -> MutexGuard<'_, V> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Tones {
    pub fn new(handle: Handle, store: Arc<dyn ToneStore>, memo_entries: usize) -> Arc<Tones> {
        Arc::new(Tones {
            handle,
            store,
            memo: Mutex::new(Memo { capacity: memo_entries.max(1), order: VecDeque::new(), tones: HashMap::new() }),
            running: Mutex::new(HashMap::new()),
            demands: Mutex::new(HashMap::new()),
            preparation: Mutex::new(None),
            hubs: Arc::new(Mutex::new(HashMap::new())),
            analyses: AtomicU64::new(0),
            suspended: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            idle: Notify::new(),
            demand_seq: AtomicU64::new(0),
            #[cfg(test)]
            delivered_changes: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn analyses(&self) -> u64 {
        self.analyses.load(Ordering::Relaxed)
    }

    pub fn listener_count(&self) -> usize {
        locked(&self.hubs).values().map(Hub::listener_count).sum()
    }

    pub async fn tone(self: &Arc<Self>, source: &MediaItem, band: f32) -> Result<Option<MediaTone>, ToneError> {
        valid_band(band)?;
        let key = self.store.key_for(&source.url, &source.headers);
        let data = self.current(&key, source).await;
        if !data.as_ref().is_some_and(|tone| tone.source == ToneSource::Pixels) {
            let seq = self.demand_seq.fetch_add(1, Ordering::Relaxed);
            locked(&self.demands).entry(key.clone()).or_insert_with(|| Demand { source: source.clone(), queried: false, seq }).queried = true;
            self.trim_demands();
            self.request(&key);
        }
        data.map(|data| data.at(band)).transpose()
    }

    /// Reads that found no pixels leave one demand each; the oldest reads beyond the memo size are forgotten.
    fn trim_demands(&self) {
        let hubs = locked(&self.hubs);
        let mut demands = locked(&self.demands);
        let mut reads: Vec<(u64, String)> = demands.iter().filter(|(key, demand)| demand.queried && !hubs.contains_key(*key)).map(|(key, demand)| (demand.seq, key.clone())).collect();
        if reads.len() <= MEMO_ENTRIES {
            return;
        }
        reads.sort_unstable();
        let excess = reads.len() - MEMO_ENTRIES;
        let running = locked(&self.running);
        for (_, key) in reads.into_iter().take(excess).filter(|(_, key)| !running.contains_key(key)) {
            demands.remove(&key);
        }
    }

    /// Cancels every preparation and waits until none runs: no callback is delivered after this returns.
    pub async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        *locked(&self.preparation) = None;
        for abort in locked(&self.running).values() { abort.abort(); }
        loop {
            let idle = self.idle.notified();
            tokio::pin!(idle);
            idle.as_mut().enable();
            if self.active.load(Ordering::SeqCst) == 0 { break; }
            idle.await;
        }
        locked(&self.demands).clear();
    }

    pub fn set_preparation(self: &Arc<Self>, preparation: Arc<dyn TonePreparation>) {
        if self.closed.load(Ordering::SeqCst) { return; }
        *locked(&self.preparation) = Some(preparation);
        let keys: Vec<_> = locked(&self.demands).keys().cloned().collect();
        for key in keys { self.request(&key); }
    }

    /// Delivers the tone at `band` once after the watch is set and again after every change of the source tone.
    pub fn subscribe(self: &Arc<Self>, source: &MediaItem, band: f32, deliver: Deliver<Option<MediaTone>>) -> Result<Subscription, ToneError> {
        valid_band(band)?;
        let key = self.store.key_for(&source.url, &source.headers);
        let pending = Arc::new(Notify::new());
        let mark = pending.clone();
        #[cfg(test)]
        let delivered_changes = self.delivered_changes.clone();
        let mut hubs = locked(&self.hubs);
        let inner = hubs.entry(key.clone()).or_insert_with(|| Hub::new(self.handle.clone())).subscribe(Arc::new(move |_: ToneSource| {
            mark.notify_one();
            #[cfg(test)]
            { delivered_changes.fetch_add(1, Ordering::SeqCst); crate::test_signal::fire(); }
        }));
        let seq = self.demand_seq.fetch_add(1, Ordering::Relaxed);
        locked(&self.demands).entry(key.clone()).or_insert_with(|| Demand { source: source.clone(), queried: false, seq });
        drop(hubs);
        pending.notify_one();
        let reader = AbortOnDrop::new(self.handle.spawn(read_changes(Arc::downgrade(self), key.clone(), source.clone(), band, pending, deliver)));
        self.request(&key);
        let hubs = self.hubs.clone();
        let tones = Arc::downgrade(self);
        Ok(Subscription::cancelling(move || {
            drop(reader);
            drop(inner);
            let mut hubs = locked(&hubs);
            if hubs.get(&key).is_some_and(|hub| hub.listener_count() == 0) {
                hubs.remove(&key);
            }
            drop(hubs);
            if let Some(tones) = tones.upgrade() { tones.cancel_unused(&key); }
        }))
    }

    fn demanded(&self, key: &str) -> bool {
        let hubs = locked(&self.hubs);
        hubs.get(key).is_some_and(|hub| hub.listener_count() > 0)
            || locked(&self.demands).get(key).is_some_and(|demand| demand.queried)
    }

    pub async fn pixels_needed(&self, source: &MediaItem) -> bool {
        let key = self.store.key_for(&source.url, &source.headers);
        self.demanded(&key) && !self.known(&key).await.is_some_and(|tone| tone.source == ToneSource::Pixels)
    }

    fn cancel_unused(&self, key: &str) {
        let hubs = locked(&self.hubs);
        let mut demands = locked(&self.demands);
        if !hubs.get(key).is_some_and(|hub| hub.listener_count() > 0) && !demands.get(key).is_some_and(|demand| demand.queried) {
            if let Some(abort) = locked(&self.running).get(key) { abort.abort(); }
            demands.remove(key);
        }
    }

    pub fn refresh(self: &Arc<Self>, url: &str) {
        let keys: Vec<_> = locked(&self.demands).iter().filter(|(_, demand)| demand.source.url == url).map(|(key, _)| key.clone()).collect();
        for key in keys { self.request(&key); }
    }

    pub fn forget(&self, url: &str) {
        let matches = |key: &str| key == url || key.strip_prefix(url).is_some_and(|suffix| suffix.starts_with('#'));
        locked(&self.demands).retain(|_, demand| demand.source.url != url);
        for (key, abort) in locked(&self.running).iter() {
            if matches(key) { abort.abort(); }
        }
        let mut memo = locked(&self.memo);
        memo.tones.retain(|key, _| !matches(key));
        memo.order.retain(|key| !matches(key));
    }

    fn request(self: &Arc<Self>, key: &str) {
        if self.suspended.load(Ordering::SeqCst) || self.closed.load(Ordering::SeqCst) { return; }
        let Some(source) = locked(&self.demands).get(key).map(|demand| demand.source.clone()) else { return };
        let Some(preparation) = locked(&self.preparation).clone() else { return };
        if !self.demanded(key) { return; }
        let mut running = locked(&self.running);
        if running.contains_key(key) { return; }
        let (abort, registration) = AbortHandle::new_pair();
        running.insert(key.to_string(), abort);
        self.active.fetch_add(1, Ordering::SeqCst);
        drop(running);
        let guard = Running { tones: self.clone(), key: key.to_string() };
        let tones = self.clone();
        self.handle.spawn(async move {
            let retry = tones.clone();
            let key = guard.key.clone();
            let cancelled = Abortable::new(async move {
                if !tones.pixels_needed(&source).await { return; }
                let Some(pixels) = preparation.prepare(source.clone()).await else { return };
                if tones.closed.load(Ordering::SeqCst) { return; }
                if pixels.width > 64 || pixels.height > 64 || !tones.pixels_needed(&source).await { return; }
                tones.analyze_pixels(source, pixels.width, pixels.height, pixels.rgba).await;
            }, registration).await.is_err();
            if !cancelled {
                if let Some(demand) = locked(&retry.demands).get_mut(&key) { demand.queried = false; }
                retry.cancel_unused(&key);
            }
            drop(guard);
            if cancelled { retry.request(&key); }
        });
    }

    async fn current(&self, key: &str, source: &MediaItem) -> Option<ToneData> {
        self.known(key).await.or_else(|| self.blurhash_tone(key, source))
    }

    async fn known(&self, key: &str) -> Option<ToneData> {
        match self.store.tone(key).await {
            Some(tone) => Some(tone),
            None => locked(&self.memo).tones.get(key).cloned(),
        }
    }

    fn blurhash_tone(&self, key: &str, source: &MediaItem) -> Option<ToneData> {
        let tone = analyze_blurhash(source.blur_hash.as_deref()?).ok()?;
        locked(&self.memo).insert(key.to_string(), tone.clone());
        Some(tone)
    }

    async fn analyze_pixels(self: Arc<Self>, source: MediaItem, width: u32, height: u32, rgba: Vec<u8>) {
        let key = self.store.key_for(&source.url, &source.headers);
        if self.known(&key).await.is_some_and(|known| known.source == ToneSource::Pixels) {
            return;
        }
        self.analyses.fetch_add(1, Ordering::Relaxed);
        let analysed = tokio::task::spawn_blocking(move || analyze(width, height, &rgba, ToneSource::Pixels)).await;
        let tone = match analysed {
            Ok(Ok(tone)) => tone,
            Ok(Err(error)) => return crate::trace!("tone", "{} rejected: {error}", crate::trace::tail(&key)),
            Err(_) => return,
        };
        if self.store.set_tone(&key, tone.clone()).await {
            locked(&self.memo).remove(&key);
        } else {
            locked(&self.memo).insert(key.clone(), tone);
        }
        self.publish(&key);
    }

    fn publish(&self, key: &str) {
        if let Some(hub) = locked(&self.hubs).get(key) {
            hub.publish(ToneSource::Pixels);
        }
        #[cfg(test)]
        crate::test_signal::fire();
    }

    #[cfg(test)]
    pub(crate) fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn delivered_changes(&self) -> u64 {
        self.delivered_changes.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    pub(crate) fn changed(&self, key: &str) {
        self.publish(key);
    }

    #[cfg(test)]
    pub(crate) fn demand_count(&self) -> usize {
        locked(&self.demands).len()
    }
}

/// The only reader of a subscription: `pending` holds at most one mark, so changes during a read cost one more read.
async fn read_changes(tones: Weak<Tones>, key: String, source: MediaItem, band: f32, pending: Arc<Notify>, deliver: Deliver<Option<MediaTone>>) {
    let mut last: Option<Option<MediaTone>> = None;
    loop {
        pending.notified().await;
        let Some(tones) = tones.upgrade() else { return };
        let value = tones.current(&key, &source).await.and_then(|data| data.at(band).ok());
        drop(tones);
        if last.as_ref() == Some(&value) { continue; }
        last = Some(value.clone());
        let deliver = deliver.clone();
        let _ = tokio::task::spawn_blocking(move || deliver(value)).await;
    }
}

impl Suspendable for Arc<Tones> {
    fn suspend(&self) {
        self.suspended.store(true, Ordering::SeqCst);
        for abort in locked(&self.running).values() { abort.abort(); }
    }

    fn resume(&self) {
        self.suspended.store(false, Ordering::SeqCst);
        let keys: Vec<_> = locked(&self.demands).keys().cloned().collect();
        for key in keys { self.request(&key); }
    }
}
