use super::byte_store::read_at;
use super::entry::{Entry, Kind, CacheStats};
use super::error::CacheError;
use super::evict::{evict_if_needed, names_where, remove_names};
use super::inner::CacheShared;
use super::load::load;
use super::naming::{bin_path, file_name};
use super::options::Options;
use super::reader::{Pin, Reader};
use super::save::{mark_dirty, write_index};
use super::verify::verified;
use super::writer::Writer;
use crate::analysis::ToneData;
use crate::limits::{Limits, LimitsError};
use crate::proxy::Meta;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

const WRITE_CHUNK: usize = 1024 * 1024;
const READ_ATTEMPTS: usize = 3;

#[derive(Clone)]
pub struct Cache {
    pub(super) inner: Arc<CacheShared>,
}

impl Cache {
    pub fn open(dir: impl Into<PathBuf>, limits: Limits) -> Cache {
        Cache::open_with(dir, limits, Options::default())
    }

    pub fn open_with(dir: impl Into<PathBuf>, limits: Limits, options: Options) -> Cache {
        let handle = tokio::runtime::Handle::current();
        let inner = Arc::new(CacheShared::new(dir.into(), limits, options, handle.clone()));
        handle.spawn(load(inner.clone()));
        Cache { inner }
    }

    pub fn is_ready(&self) -> bool {
        self.inner.is_ready()
    }

    pub async fn ready(&self) {
        let _ = self.inner.ready.subscribe().wait_for(|ready| *ready).await;
    }

    pub async fn flush(&self) {
        write_index(&self.inner).await;
    }

    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.get_range(key, 0, u64::MAX).await
    }

    pub async fn get_range(&self, key: &str, start: u64, len: u64) -> Option<Vec<u8>> {
        for _ in 0..READ_ATTEMPTS {
            let (pin, size) = self.pinned(key).await?;
            let path = bin_path(&self.inner.dir, &file_name(key));
            let wanted = len.min(size.saturating_sub(start));
            let read = tokio::task::spawn_blocking(move || {
                let _pin = pin;
                let file = std::fs::File::open(&path).ok()?;
                Some(if file.metadata().ok()?.len() == size { read_at(&file, start, wanted) } else { None })
            })
            .await;
            match read {
                Ok(Some(Some(bytes))) => return Some(bytes),
                Ok(Some(None)) => continue,
                _ => break,
            }
        }
        self.forget_missing(key);
        None
    }

    async fn pinned(&self, key: &str) -> Option<(Pin, u64)> {
        if !self.is_ready() {
            return None;
        }
        let name = file_name(key);
        verified(&self.inner, &name).await?;
        Pin::acquire(&self.inner, &name)
    }

    fn forget_missing(&self, key: &str) {
        let name = file_name(key);
        if !bin_path(&self.inner.dir, &name).exists() && self.inner.state().take(&name).is_some() {
            mark_dirty(&self.inner);
        }
    }

    pub async fn open_reader(&self, key: &str) -> Option<Reader> {
        if !self.is_ready() {
            return None;
        }
        let name = file_name(key);
        verified(&self.inner, &name).await?;
        let (pin, size) = Pin::acquire(&self.inner, &name)?;
        match tokio::fs::File::open(bin_path(&self.inner.dir, &name)).await {
            Ok(file) => Some(Reader::new(pin, file, size)),
            Err(_) => {
                drop(pin);
                self.inner.state().take(&name);
                mark_dirty(&self.inner);
                None
            }
        }
    }

    pub async fn put(&self, key: &str, kind: Kind, data: &[u8]) -> Result<(), CacheError> {
        let mut writer = self.begin_write(key, kind).await?;
        for chunk in data.chunks(WRITE_CHUNK) {
            writer.write(chunk).await?;
        }
        writer.commit().await
    }

    pub async fn begin_write(&self, key: &str, kind: Kind) -> Result<Writer, CacheError> {
        Writer::start(self.inner.clone(), key, kind).await
    }

    pub async fn len(&self, key: &str) -> Option<u64> {
        self.entry_of(key).await.map(|entry| entry.size)
    }

    pub async fn meta(&self, key: &str) -> Option<Meta> {
        self.entry_of(key).await.map(|entry| entry.meta)
    }

    async fn entry_of(&self, key: &str) -> Option<Entry> {
        if !self.is_ready() {
            return None;
        }
        verified(&self.inner, &file_name(key)).await
    }

    pub fn tone(&self, key: &str) -> Option<ToneData> {
        if !self.is_ready() {
            return None;
        }
        self.inner.state().entry_state(&file_name(key)).and_then(|(_, entry, _)| entry.tone)
    }

    pub fn set_tone(&self, key: &str, tone: ToneData) -> bool {
        if !self.is_ready() || !self.inner.state().set_tone(&file_name(key), tone) {
            return false;
        }
        mark_dirty(&self.inner);
        true
    }

    pub async fn contains(&self, key: &str) -> bool {
        self.entry_of(key).await.is_some()
    }

    pub async fn remove(&self, key: &str) -> bool {
        self.ready().await;
        remove_names(&self.inner, vec![file_name(key)]).await == 1
    }

    pub async fn purge_prefix(&self, prefix: &str) -> usize {
        self.ready().await;
        let names = names_where(&self.inner, |key, _| key.starts_with(prefix)).await;
        remove_names(&self.inner, names).await
    }

    pub async fn clear(&self, kind: Option<Kind>) -> usize {
        self.ready().await;
        let names = names_where(&self.inner, |_, entry_kind| kind.is_none_or(|wanted| entry_kind == wanted)).await;
        remove_names(&self.inner, names).await
    }

    pub async fn stats(&self) -> CacheStats {
        let mut stats = self.inner.state().stats();
        stats.max_hold = Duration::from_nanos(self.inner.max_hold.load(Ordering::Relaxed));
        stats.eviction_backlog = self.inner.eviction_backlog.load(Ordering::Relaxed);
        stats
    }

    pub async fn set_limits(&self, limits: Limits) -> Result<(), LimitsError> {
        limits.validate()?;
        self.inner.state().limits = limits;
        evict_if_needed(&self.inner).await;
        Ok(())
    }

    pub fn reset_max_hold(&self) {
        self.inner.max_hold.store(0, Ordering::Relaxed);
    }

    pub async fn evict_now(&self) {
        evict_if_needed(&self.inner).await;
    }
}
