use super::entry::Kind;
use super::store::Cache;
use super::writer::Writer;
use crate::proxy::{ByteStore, Meta, StoreWriter};
use async_trait::async_trait;
use std::collections::HashMap;
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::sync::{Arc, Mutex, PoisonError};
use url::Url;

const IMAGE_EXTENSIONS: [&str; 8] = ["jpg", "jpeg", "png", "webp", "gif", "heic", "heif", "avif"];
const AUDIO_EXTENSIONS: [&str; 6] = ["mp3", "m4a", "aac", "wav", "ogg", "flac"];

pub type Classify = Arc<dyn Fn(&str, &Meta) -> Kind + Send + Sync>;

type OpenWrites = Arc<Mutex<HashMap<String, Arc<File>>>>;

pub fn classify_kind(key: &str, meta: &Meta) -> Kind {
    let content_type = meta.content_type.as_deref().unwrap_or_default().to_ascii_lowercase();
    if content_type.starts_with("image/") {
        return Kind::Image;
    }
    if content_type.starts_with("audio/") {
        return Kind::Audio;
    }
    if content_type.starts_with("video/") || content_type.contains("mpegurl") {
        return Kind::Stream;
    }
    match extension_of(key).as_deref() {
        Some(extension) if IMAGE_EXTENSIONS.contains(&extension) => Kind::Image,
        Some(extension) if AUDIO_EXTENSIONS.contains(&extension) => Kind::Audio,
        _ => Kind::Stream,
    }
}

fn extension_of(key: &str) -> Option<String> {
    let parsed = Url::parse(key).ok()?;
    let segment = parsed.path_segments()?.next_back()?;
    segment.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase())
}

pub(super) fn read_at(file: &File, start: u64, len: u64) -> Option<Vec<u8>> {
    let mut buffer = vec![0u8; len as usize];
    let mut filled = 0;
    while filled < buffer.len() {
        let read = file.read_at(&mut buffer[filled..], start + filled as u64).ok()?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    buffer.truncate(filled);
    Some(buffer)
}

#[derive(Clone)]
pub struct CacheStore {
    cache: Cache,
    classify: Classify,
    open: OpenWrites,
}

impl CacheStore {
    pub fn new(cache: Cache, classify: Classify) -> CacheStore {
        CacheStore { cache, classify, open: Arc::new(Mutex::new(HashMap::new())) }
    }

    async fn read_open(&self, key: &str, start: u64, len: u64) -> Option<Vec<u8>> {
        let file = self.open.lock().unwrap_or_else(PoisonError::into_inner).get(key).cloned()?;
        tokio::task::spawn_blocking(move || read_at(&file, start, len)).await.ok()?
    }
}

#[async_trait]
impl ByteStore for CacheStore {
    async fn len(&self, key: &str) -> Option<u64> {
        self.cache.ready().await;
        self.cache.len(key).await
    }

    async fn contains(&self, key: &str) -> bool {
        self.cache.ready().await;
        self.cache.contains(key).await
    }

    async fn get_range(&self, key: &str, start: u64, len: u64) -> Option<Vec<u8>> {
        self.cache.ready().await;
        if let Some(bytes) = self.cache.get_range(key, start, len).await {
            return Some(bytes);
        }
        if let Some(bytes) = self.read_open(key, start, len).await {
            return Some(bytes);
        }
        self.cache.get_range(key, start, len).await
    }

    async fn meta(&self, key: &str) -> Option<Meta> {
        self.cache.ready().await;
        self.cache.meta(key).await
    }

    async fn begin_write(&self, key: &str, meta: Meta) -> Option<Box<dyn StoreWriter>> {
        let kind = (self.classify)(key, &meta);
        let writer = self.cache.begin_write(key, kind).await.ok()?.with_meta(meta);
        let file = Arc::new(File::open(writer.part()).ok()?);
        self.open.lock().unwrap_or_else(PoisonError::into_inner).insert(key.to_string(), file.clone());
        let registration = Registration { open: self.open.clone(), key: key.to_string(), file };
        Some(Box::new(OpenWrite { writer: Some(writer), _registration: registration }))
    }

    async fn remove(&self, key: &str) {
        self.cache.remove(key).await;
    }
}

struct Registration {
    open: OpenWrites,
    key: String,
    file: Arc<File>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if open.get(&self.key).is_some_and(|current| Arc::ptr_eq(current, &self.file)) {
            open.remove(&self.key);
        }
    }
}

struct OpenWrite {
    writer: Option<Writer>,
    _registration: Registration,
}

#[async_trait]
impl StoreWriter for OpenWrite {
    async fn append(&mut self, bytes: &[u8]) -> bool {
        match self.writer.as_mut() {
            Some(writer) => writer.write(bytes).await.is_ok(),
            None => false,
        }
    }

    async fn commit(mut self: Box<Self>) -> bool {
        match self.writer.take() {
            Some(writer) => writer.commit().await.is_ok(),
            None => false,
        }
    }

    async fn commit_exact(mut self: Box<Self>, length: u64) -> bool {
        match self.writer.take() {
            Some(writer) => writer.commit_exact(length).await.is_ok(),
            None => false,
        }
    }

    async fn abort(self: Box<Self>) {}
}
