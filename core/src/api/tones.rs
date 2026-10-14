use super::hub::{Deliver, Hub, Subscription};
use super::source::MediaItem;
use crate::analysis::{analyze, analyze_blurhash, valid_band, MediaTone, ToneData, ToneError, ToneSource};
use crate::cache::Cache;
use crate::proxy::block_key;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::runtime::Handle;

pub const MEMO_ENTRIES: usize = 256;

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
    running: Mutex<HashSet<String>>,
    hubs: Hubs,
    analyses: AtomicU64,
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
            running: Mutex::new(HashSet::new()),
            hubs: Arc::new(Mutex::new(HashMap::new())),
            analyses: AtomicU64::new(0),
        })
    }

    pub fn analyses(&self) -> u64 {
        self.analyses.load(Ordering::Relaxed)
    }

    pub fn listener_count(&self) -> usize {
        locked(&self.hubs).values().map(Hub::listener_count).sum()
    }

    pub async fn tone(&self, source: &MediaItem, band: f32) -> Result<Option<MediaTone>, ToneError> {
        valid_band(band)?;
        let key = self.store.key_for(&source.url, &source.headers);
        let Some(data) = self.known(&key).await.or_else(|| self.from_blurhash(&key, source)) else {
            return Ok(None);
        };
        data.at(band).map(Some)
    }

    pub fn submit(self: &Arc<Self>, source: MediaItem, width: u32, height: u32, rgba: Vec<u8>) {
        let tones = self.clone();
        self.handle.spawn(async move { tones.analyze_pixels(source, width, height, rgba).await });
    }

    pub fn subscribe(&self, url: &str, deliver: Deliver<ToneSource>) -> Subscription {
        let inner = locked(&self.hubs).entry(url.to_string()).or_insert_with(|| Hub::new(self.handle.clone())).subscribe(deliver);
        let (hubs, url) = (self.hubs.clone(), url.to_string());
        Subscription::cancelling(move || {
            drop(inner);
            let mut hubs = locked(&hubs);
            if hubs.get(&url).is_some_and(|hub| hub.listener_count() == 0) {
                hubs.remove(&url);
            }
        })
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
        if !locked(&self.running).insert(key.clone()) {
            return;
        }
        let _running = Running { tones: self.clone(), key: key.clone() };
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
            locked(&self.memo).insert(key, tone);
        }
        if let Some(hub) = locked(&self.hubs).get(&source.url) {
            hub.publish(ToneSource::Pixels);
        }
    }
}
