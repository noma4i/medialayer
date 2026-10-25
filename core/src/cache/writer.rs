use super::entry::{Check, Entry, Kind};
use super::error::CacheError;
use super::evict::trigger_eviction;
use super::inner::{delete_files, CacheShared};
use crate::proxy::Meta;
use super::naming::{bin_path, file_name, part_path};
use super::reservation::SpaceReservation;
use super::save::mark_dirty;
use super::verify::{sum_of, SEGMENT};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::io::Write;
use tokio::runtime::Handle;
use tokio::sync::oneshot;

struct Registration {
    generation: u64,
    inner: Arc<CacheShared>,
    name: String,
    part: PathBuf,
}

impl Registration {
    fn new(inner: &Arc<CacheShared>, name: &str, key: &str, kind: Kind, part: &Path) -> Registration {
        let mut state = inner.state();
        let generation = state.start_writing(name, key, kind);
        state.live_parts.insert(part.to_path_buf());
        Registration { generation, inner: inner.clone(), name: name.to_string(), part: part.to_path_buf() }
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
    inner: Arc<CacheShared>,
    key: String,
    name: String,
    kind: Kind,
    meta: Meta,
    part: PathBuf,
    _registration: Registration,
    file: Option<Arc<std::fs::File>>,
    reservation: SpaceReservation,
    hasher: Sha256,
    filled: usize,
    sums: Vec<u64>,
    renamed: bool,
    #[cfg(test)]
    io_calls: Arc<(std::sync::atomic::AtomicUsize, std::sync::atomic::AtomicUsize)>,
}

impl Writer {
    pub(super) async fn start(inner: Arc<CacheShared>, key: &str, kind: Kind) -> Result<Writer, CacheError> {
        tokio::fs::create_dir_all(&inner.dir).await?;
        let name = file_name(key);
        let part = part_path(&inner.dir, &name, inner.part_sequence.fetch_add(1, Ordering::SeqCst));
        let registration = Registration::new(&inner, &name, key, kind, &part);
        let reservation = SpaceReservation::new(inner.clone());
        let mut writer = Writer { inner, key: key.to_string(), name, kind, meta: Meta::default(), part, _registration: registration, file: None, reservation, hasher: Sha256::new(), filled: 0, sums: Vec::new(), renamed: false, #[cfg(test)] io_calls: Arc::default() };
        writer.file = Some(create_part(&writer.inner.handle, &writer.part).await?);
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
        let file = self.file.as_ref().ok_or_else(|| CacheError::Io("writer is closed".to_string()))?.clone();
        #[cfg(test)]
        self.io_calls.0.fetch_add(1, Ordering::Relaxed);
        let bytes = data.to_vec();
        #[cfg(test)]
        self.inner.copied_bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        tokio::task::spawn_blocking(move || (&*file).write_all(&bytes)).await.map_err(|error| CacheError::Io(error.to_string()))??;
        self.absorb(data);
        Ok(())
    }

    fn absorb(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            let take = (SEGMENT - self.filled).min(data.len());
            self.hasher.update(&data[..take]);
            self.filled += take;
            data = &data[take..];
            if self.filled == SEGMENT {
                self.sums.push(sum_of(std::mem::take(&mut self.hasher)));
                self.filled = 0;
            }
        }
    }

    pub async fn abort(mut self) {
        self.file.take();
        self.renamed = true;
        delete_files(vec![self.part.clone()]).await;
    }

    pub async fn commit_exact(self, length: u64) -> Result<(), CacheError> {
        if self.reservation.written() != length {
            return Err(CacheError::Io(format!("body has {} of {length} bytes", self.reservation.written())));
        }
        self.commit().await
    }

    pub async fn commit(mut self) -> Result<(), CacheError> {
        let file = self.file.take().ok_or_else(|| CacheError::Io("writer is closed".to_string()))?;
        #[cfg(test)]
        self.io_calls.1.fetch_add(1, Ordering::Relaxed);
        tokio::task::spawn_blocking(move || (&*file).flush()).await.map_err(|error| CacheError::Io(error.to_string()))??;
        let stripe = self.inner.stripe(&self.name).clone().lock_owned().await;
        let handle = self.inner.handle.clone();
        handle.spawn(self.finish(stripe)).await.map_err(|error| CacheError::Io(error.to_string()))?
    }

    async fn finish(mut self, _stripe: tokio::sync::OwnedMutexGuard<()>) -> Result<(), CacheError> {
        let inner = self.inner.clone();
        if !inner.state().writer_is_current(&self.name, self._registration.generation) {
            return Err(CacheError::Purged);
        }
        tokio::fs::rename(&self.part, bin_path(&inner.dir, &self.name)).await?;
        self.renamed = true;
        let now = inner.now();
        {
            let mut state = inner.state();
            let size = self.reservation.written();
            self.reservation.settle(&mut state);
            let mut sums = std::mem::take(&mut self.sums);
            if self.filled > 0 {
                sums.push(sum_of(std::mem::take(&mut self.hasher)));
            }
            let check = Check(sums.into());
            let entry = Entry { size, check, kind: self.kind, created: now, last_access: now, hits: 0, meta: self.meta.clone(), tone: None, image_size: None };
            state.insert(self.name.clone(), self.key.clone(), entry, true);
        }
        mark_dirty(&inner);
        trigger_eviction(&inner);
        Ok(())
    }
}

#[cfg(test)]
#[path = "writer_work_tests.rs"]
mod chunk_work_tests;

async fn create_part(handle: &Handle, part: &Path) -> Result<Arc<std::fs::File>, CacheError> {
    let (sender, receiver) = oneshot::channel();
    let part = part.to_path_buf();
    handle.spawn(async move {
        let created = match tokio::fs::File::create(&part).await {
            Ok(file) => Ok(Arc::new(file.into_std().await)),
            Err(error) => Err(error),
        };
        if let Err(Ok(abandoned)) = sender.send(created) {
            drop(abandoned);
            delete_files(vec![part]).await;
        }
    });
    let created = receiver.await.map_err(|_| CacheError::Io("part creation was dropped".to_string()))?;
    Ok(created?)
}

impl Drop for Writer {
    fn drop(&mut self) {
        #[cfg(test)]
        self.inner.writer_drops.fetch_add(1, Ordering::Relaxed);
        if !self.renamed {
            let part = self.part.clone();
            self.inner.handle.spawn(delete_files(vec![part]));
        }
    }
}
