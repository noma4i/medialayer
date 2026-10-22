use super::hub::{Deliver, Hub, Subscription};
use super::ports::Suspendable;
use super::source::MediaItem;
use crate::analysis::{analyze, analyze_blurhash, valid_band, MediaTone, ToneData, ToneError, ToneSource};
use crate::cache::Cache;
use crate::proxy::block_key;
use async_trait::async_trait;
use futures_util::future::{AbortHandle, Abortable};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::runtime::Handle;

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
}

struct Running {
    tones: Arc<Tones>,
    key: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        locked(&self.tones.running).remove(&self.key);
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
        let data = self.known(&key).await.or_else(|| self.from_blurhash(&key, source));
        if !data.as_ref().is_some_and(|tone| tone.source == ToneSource::Pixels) {
            locked(&self.demands).entry(key.clone()).or_insert_with(|| Demand { source: source.clone(), queried: false }).queried = true;
            self.request(&key);
        }
        data.map(|data| data.at(band)).transpose()
    }

    pub fn set_preparation(self: &Arc<Self>, preparation: Arc<dyn TonePreparation>) {
        *locked(&self.preparation) = Some(preparation);
        let keys: Vec<_> = locked(&self.demands).keys().cloned().collect();
        for key in keys { self.request(&key); }
    }

    pub fn subscribe(self: &Arc<Self>, source: &MediaItem, deliver: Deliver<ToneSource>) -> Subscription {
        let key = self.store.key_for(&source.url, &source.headers);
        let mut hubs = locked(&self.hubs);
        let inner = hubs.entry(key.clone()).or_insert_with(|| Hub::new(self.handle.clone())).subscribe(deliver);
        locked(&self.demands).entry(key.clone()).or_insert_with(|| Demand { source: source.clone(), queried: false });
        drop(hubs);
        self.request(&key);
        let hubs = self.hubs.clone();
        let tones = Arc::downgrade(self);
        Subscription::cancelling(move || {
            drop(inner);
            let mut hubs = locked(&hubs);
            if hubs.get(&key).is_some_and(|hub| hub.listener_count() == 0) {
                hubs.remove(&key);
            }
            drop(hubs);
            if let Some(tones) = tones.upgrade() { tones.cancel_unused(&key); }
        })
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
        if self.suspended.load(Ordering::SeqCst) { return; }
        let Some(source) = locked(&self.demands).get(key).map(|demand| demand.source.clone()) else { return };
        let Some(preparation) = locked(&self.preparation).clone() else { return };
        if !self.demanded(key) { return; }
        let mut running = locked(&self.running);
        if running.contains_key(key) { return; }
        let (abort, registration) = AbortHandle::new_pair();
        running.insert(key.to_string(), abort);
        drop(running);
        let guard = Running { tones: self.clone(), key: key.to_string() };
        let tones = self.clone();
        self.handle.spawn(async move {
            let retry = tones.clone();
            let key = guard.key.clone();
            let cancelled = Abortable::new(async move {
                if !tones.pixels_needed(&source).await { return; }
                let Some(pixels) = preparation.prepare(source.clone()).await else { return };
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

    async fn known(&self, key: &str) -> Option<ToneData> {
        match self.store.tone(key).await {
            Some(tone) => Some(tone),
            None => locked(&self.memo).tones.get(key).cloned(),
        }
    }

    fn from_blurhash(&self, key: &str, source: &MediaItem) -> Option<ToneData> {
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
        if let Some(hub) = locked(&self.hubs).get(&key) {
            hub.publish(ToneSource::Pixels);
        }
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
