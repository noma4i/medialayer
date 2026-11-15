use super::entry::Kind;
use super::store::Cache;
use super::writer::Writer;
use crate::proxy::{ByteStore, Meta, StoreReader, StoreWriter};
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

fn open_part(inner: &super::inner::CacheShared, part: &std::path::Path) -> Option<File> {
    #[cfg(test)]
    {
        let paused = inner.write_open_pause.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some((started, release)) = paused {
            let _ = started.send(());
            let _ = release.blocking_recv();
        }
    }
    let file = File::open(part).ok()?;
    #[cfg(test)]
    inner.write_open_threads.lock().unwrap_or_else(PoisonError::into_inner).push(std::thread::current().id());
    let _ = inner;
    Some(file)
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

    #[cfg(test)]
    pub(super) fn open_writes(&self) -> usize {
        self.open.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

}

#[async_trait]
impl ByteStore for CacheStore {
    async fn open_reader(&self, key: &str) -> Option<Box<dyn StoreReader>> {
        self.cache.ready().await;
        if let Some(reader) = self.cache.open_reader(key).await {
            return Some(Box::new(reader));
        }
        let file = self.open.lock().unwrap_or_else(PoisonError::into_inner).get(key).cloned();
        if let Some(file) = file {
            return Some(Box::new(super::reader::Reader::writing(file, self.cache.inner.chunk_size)));
        }
        self.cache.open_reader(key).await.map(|reader| Box::new(reader) as Box<dyn StoreReader>)
    }
    async fn keep(&self, key: &str) {
        self.cache.keep(key);
    }

    async fn len(&self, key: &str) -> Option<u64> {
        self.cache.ready().await;
        self.cache.len(key).await
    }

    async fn contains(&self, key: &str) -> bool {
        self.cache.ready().await;
        self.cache.contains(key).await
    }

    async fn version(&self, key: &str) -> Option<(u64, u64)> {
        self.cache.ready().await;
        self.cache.version(key).await
    }

    async fn get_range(&self, key: &str, start: u64, len: u64) -> Option<Vec<u8>> {
        self.open_reader(key).await?.get_range(start, len).await
    }

    async fn meta(&self, key: &str) -> Option<Meta> {
        self.cache.ready().await;
        self.cache.meta(key).await
    }

    async fn begin_write(&self, key: &str, meta: Meta) -> Option<Box<dyn StoreWriter>> {
        let kind = (self.classify)(key, &meta);
        let writer = self.cache.begin_write(key, kind).await.ok()?.with_meta(meta);
        let inner = self.cache.inner.clone();
        let (writer, file) = tokio::task::spawn_blocking(move || {
            let file = open_part(&inner, writer.part())?;
            Some((writer, file))
        })
        .await
        .ok()??;
        let file = Arc::new(file);
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
            #[cfg(test)]
            crate::test_signal::fire();
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
