use super::error::CacheError;
use super::inner::CacheShared;
use std::io::SeekFrom;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

pub struct Pin {
    inner: Arc<CacheShared>,
    name: String,
}

impl Pin {
    pub fn acquire(inner: &Arc<CacheShared>, name: &str) -> Option<(Pin, u64)> {
        let size = inner.state().pin(name, inner.now())?;
        Some((Pin { inner: inner.clone(), name: name.to_string() }, size))
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        self.inner.state().unpin(&self.name);
    }
}

pub struct Reader {
    pin: Pin,
    file: tokio::fs::File,
    size: u64,
}

impl Reader {
    pub(super) fn new(pin: Pin, file: tokio::fs::File, size: u64) -> Reader {
        Reader { pin, file, size }
    }

    pub fn len(&self) -> u64 {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub async fn read_chunk(&mut self) -> Result<Vec<u8>, CacheError> {
        let mut buffer = vec![0u8; self.pin.inner.chunk_size];
        let read = self.file.read(&mut buffer).await?;
        buffer.truncate(read);
        Ok(buffer)
    }

    pub(super) async fn read_all(&mut self) -> Result<Vec<u8>, CacheError> {
        let mut buffer = Vec::with_capacity(self.size as usize);
        self.file.read_to_end(&mut buffer).await?;
        Ok(buffer)
    }

    pub(super) async fn read_range(&mut self, start: u64, len: u64) -> Result<Vec<u8>, CacheError> {
        self.file.seek(SeekFrom::Start(start)).await?;
        let mut buffer = Vec::new();
        (&mut self.file).take(len).read_to_end(&mut buffer).await?;
        Ok(buffer)
    }
}
