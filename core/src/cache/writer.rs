use super::entry::{Entry, Kind};
use super::error::CacheError;
use super::evict::trigger_eviction;
use super::inner::{delete_files, Inner};
use crate::proxy::Meta;
use super::naming::{bin_path, file_name, part_path};
use super::reservation::Reservation;
use super::save::mark_dirty;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

struct Registration {
    inner: Arc<Inner>,
    name: String,
    part: PathBuf,
}

impl Registration {
    fn new(inner: &Arc<Inner>, name: &str, part: &Path) -> Registration {
        let mut state = inner.state();
        state.start_writing(name);
        state.live_parts.insert(part.to_path_buf());
        Registration { inner: inner.clone(), name: name.to_string(), part: part.to_path_buf() }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut state = self.inner.state();
        state.stop_writing(&self.name);
        state.live_parts.remove(&self.part);
    }
}

pub struct Writer {
    inner: Arc<Inner>,
    key: String,
    name: String,
    kind: Kind,
    meta: Meta,
    part: PathBuf,
    _registration: Registration,
    file: Option<tokio::fs::File>,
    reservation: Reservation,
    renamed: bool,
}

impl Writer {
    pub(super) async fn start(inner: Arc<Inner>, key: &str, kind: Kind) -> Result<Writer, CacheError> {
        tokio::fs::create_dir_all(&inner.dir).await?;
        let name = file_name(key);
        let part = part_path(&inner.dir, &name, inner.part_sequence.fetch_add(1, Ordering::SeqCst));
        let registration = Registration::new(&inner, &name, &part);
        let reservation = Reservation::new(inner.clone());
        let mut writer = Writer { inner, key: key.to_string(), name, kind, meta: Meta::default(), part, _registration: registration, file: None, reservation, renamed: false };
        writer.file = Some(tokio::fs::File::create(&writer.part).await?);
        Ok(writer)
    }

    pub fn with_meta(mut self, meta: Meta) -> Writer {
        self.meta = meta;
        self
    }

    pub(super) fn part(&self) -> &std::path::Path {
        &self.part
    }

    pub async fn write(&mut self, data: &[u8]) -> Result<(), CacheError> {
        self.reservation.grow(data.len() as u64).await?;
        let file = self.file.as_mut().ok_or_else(|| CacheError::Io("writer is closed".to_string()))?;
        file.write_all(data).await?;
        file.flush().await?;
        Ok(())
    }

    pub async fn abort(mut self) {
        self.file.take();
        self.renamed = true;
        delete_files(vec![self.part.clone()]).await;
    }

    pub async fn commit(mut self) -> Result<(), CacheError> {
        let mut file = self.file.take().ok_or_else(|| CacheError::Io("writer is closed".to_string()))?;
        file.flush().await?;
        drop(file);
        let stripe = self.inner.stripe(&self.name).clone().lock_owned().await;
        let handle = self.inner.handle.clone();
        handle.spawn(self.finish(stripe)).await.map_err(|error| CacheError::Io(error.to_string()))?
    }

    async fn finish(mut self, _stripe: tokio::sync::OwnedMutexGuard<()>) -> Result<(), CacheError> {
        let inner = self.inner.clone();
        tokio::fs::rename(&self.part, bin_path(&inner.dir, &self.name)).await?;
        self.renamed = true;
        let now = inner.now();
        {
            let mut state = inner.state();
            let size = self.reservation.held();
            self.reservation.settle(&mut state);
            let entry = Entry { size, kind: self.kind, created: now, last_access: now, hits: 0, meta: self.meta.clone() };
            state.insert(self.name.clone(), self.key.clone(), entry);
        }
        mark_dirty(&inner);
        trigger_eviction(&inner);
        Ok(())
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        if !self.renamed {
            let part = self.part.clone();
            self.inner.handle.spawn(delete_files(vec![part]));
        }
    }
}
