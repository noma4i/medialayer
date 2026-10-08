use super::byte_store::{ByteStore, Meta, StoreWriter};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

struct Entry {
    data: Vec<u8>,
    complete: bool,
    meta: Meta,
}

#[derive(Default)]
struct Shared {
    entries: Mutex<HashMap<String, Entry>>,
    open_writers: AtomicUsize,
    commits: AtomicUsize,
    aborts: AtomicUsize,
    refuse_writes: AtomicBool,
}

#[derive(Clone, Default)]
pub struct MemoryStore {
    shared: Arc<Shared>,
}

impl MemoryStore {
    pub fn new() -> Self {
        MemoryStore::default()
    }

    pub fn seed(&self, key: &str, data: &[u8]) {
        self.entries().insert(key.to_string(), Entry { data: data.to_vec(), complete: true, meta: Meta::default() });
    }

    pub fn bytes(&self, key: &str) -> Option<Vec<u8>> {
        self.entries().get(key).filter(|entry| entry.complete).map(|entry| entry.data.clone())
    }

    pub fn clear(&self) {
        self.entries().clear();
    }

    pub fn count(&self) -> usize {
        self.entries().len()
    }

    pub fn open_writers(&self) -> usize {
        self.shared.open_writers.load(Ordering::SeqCst)
    }

    pub fn commits(&self) -> usize {
        self.shared.commits.load(Ordering::SeqCst)
    }

    pub fn aborts(&self) -> usize {
        self.shared.aborts.load(Ordering::SeqCst)
    }

    pub fn set_refuse_writes(&self, refuse: bool) {
        self.shared.refuse_writes.store(refuse, Ordering::SeqCst);
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.shared.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct MemoryWriter {
    shared: Arc<Shared>,
    key: String,
}

#[async_trait]
impl ByteStore for MemoryStore {
    async fn len(&self, key: &str) -> Option<u64> {
        self.entries().get(key).filter(|entry| entry.complete).map(|entry| entry.data.len() as u64)
    }

    async fn contains(&self, key: &str) -> bool {
        self.len(key).await.is_some()
    }

    async fn get_range(&self, key: &str, start: u64, len: u64) -> Option<Vec<u8>> {
        let entries = self.entries();
        let entry = entries.get(key)?;
        let from = (start as usize).min(entry.data.len());
        let to = from.saturating_add(len as usize).min(entry.data.len());
        Some(entry.data[from..to].to_vec())
    }

    async fn meta(&self, key: &str) -> Option<Meta> {
        self.entries().get(key).filter(|entry| entry.complete).map(|entry| entry.meta.clone())
    }

    async fn begin_write(&self, key: &str, meta: Meta) -> Option<Box<dyn StoreWriter>> {
        if self.shared.refuse_writes.load(Ordering::SeqCst) {
            return None;
        }
        self.entries().insert(key.to_string(), Entry { data: Vec::new(), complete: false, meta });
        self.shared.open_writers.fetch_add(1, Ordering::SeqCst);
        Some(Box::new(MemoryWriter { shared: self.shared.clone(), key: key.to_string() }))
    }

    async fn remove(&self, key: &str) {
        self.entries().remove(key);
    }
}

#[async_trait]
impl StoreWriter for MemoryWriter {
    async fn append(&mut self, bytes: &[u8]) -> bool {
        let mut entries = self.shared.entries.lock().unwrap_or_else(PoisonError::into_inner);
        match entries.get_mut(&self.key) {
            Some(entry) => {
                entry.data.extend_from_slice(bytes);
                true
            }
            None => false,
        }
    }

    async fn commit(self: Box<Self>) -> bool {
        let mut entries = self.shared.entries.lock().unwrap_or_else(PoisonError::into_inner);
        self.shared.open_writers.fetch_sub(1, Ordering::SeqCst);
        match entries.get_mut(&self.key) {
            Some(entry) => {
                entry.complete = true;
                self.shared.commits.fetch_add(1, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    async fn abort(self: Box<Self>) {
        let mut entries = self.shared.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.get(&self.key).is_some_and(|entry| !entry.complete) {
            entries.remove(&self.key);
        }
        self.shared.open_writers.fetch_sub(1, Ordering::SeqCst);
        self.shared.aborts.fetch_add(1, Ordering::SeqCst);
    }
}
