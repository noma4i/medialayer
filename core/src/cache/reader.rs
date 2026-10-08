use super::error::CacheError;
use super::inner::Inner;
use std::io::SeekFrom;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

pub struct Reader {
    inner: Arc<Inner>,
    name: String,
    file: tokio::fs::File,
    size: u64,
}

impl Reader {
    pub(super) fn new(inner: Arc<Inner>, name: String, file: tokio::fs::File, size: u64) -> Reader {
        Reader { inner, name, file, size }
    }

    pub fn len(&self) -> u64 {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub async fn read_chunk(&mut self) -> Result<Vec<u8>, CacheError> {
        let mut buffer = vec![0u8; self.inner.chunk_size];
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

impl Drop for Reader {
    fn drop(&mut self) {
        self.inner.state().unpin(&self.name);
    }
}
