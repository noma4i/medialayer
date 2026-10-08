use super::entry::{Entry, Kind};
use super::error::CacheError;
use super::evict::{release, reserve, trigger_eviction};
use super::inner::{delete_files, Inner};
use super::naming::{bin_path, file_name, part_path};
use super::save::mark_dirty;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

pub struct Writer {
    inner: Arc<Inner>,
    key: String,
    name: String,
    kind: Kind,
    part: PathBuf,
    file: Option<tokio::fs::File>,
    size: u64,
    renamed: bool,
}

impl Writer {
    pub(super) async fn start(inner: Arc<Inner>, key: &str, kind: Kind) -> Result<Writer, CacheError> {
        tokio::fs::create_dir_all(&inner.dir).await?;
        let name = file_name(key);
        let part = part_path(&inner.dir, &name, inner.part_sequence.fetch_add(1, Ordering::SeqCst));
        {
            let mut state = inner.state();
            state.start_writing(&name);
            state.live_parts.insert(part.clone());
        }
        let mut writer = Writer { inner, key: key.to_string(), name, kind, part, file: None, size: 0, renamed: false };
        writer.file = Some(tokio::fs::File::create(&writer.part).await?);
        Ok(writer)
    }

    pub async fn write(&mut self, data: &[u8]) -> Result<(), CacheError> {
        if self.size + data.len() as u64 > self.inner.state().limits.budget {
            return Err(CacheError::NotEnoughSpace);
        }
        let file = self.file.as_mut().ok_or_else(|| CacheError::Io("writer is closed".to_string()))?;
        file.write_all(data).await?;
        self.size += data.len() as u64;
        Ok(())
    }

    pub async fn commit(mut self) -> Result<(), CacheError> {
        let mut file = self.file.take().ok_or_else(|| CacheError::Io("writer is closed".to_string()))?;
        file.flush().await?;
        drop(file);
        let inner = self.inner.clone();
        reserve(&inner, &self.name, self.size).await?;
        let _stripe = inner.stripe(&self.name).lock().await;
        if let Err(error) = tokio::fs::rename(&self.part, bin_path(&inner.dir, &self.name)).await {
            release(&inner, self.size);
            return Err(error.into());
        }
        self.renamed = true;
        let now = inner.now();
        {
            let mut state = inner.state();
            state.reserved -= self.size;
            let entry = Entry { size: self.size, kind: self.kind, created: now, last_access: now, hits: 0 };
            state.insert(self.name.clone(), self.key.clone(), entry);
        }
        mark_dirty(&inner);
        trigger_eviction(&inner);
        Ok(())
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        {
            let mut state = self.inner.state();
            state.stop_writing(&self.name);
            state.live_parts.remove(&self.part);
        }
        if !self.renamed {
            let part = self.part.clone();
            self.inner.handle.spawn(delete_files(vec![part]));
        }
    }
}
