use super::error::CacheError;
use super::inner::CacheShared;
use std::sync::Arc;
use std::fs::File;

pub struct Pin {
    inner: Arc<CacheShared>,
    name: String,
    version: u64,
}

impl Pin {
    pub fn acquire(inner: &Arc<CacheShared>, name: &str) -> Option<(Pin, u64)> {
        let (size, version) = inner.state().pin(name, inner.now())?;
        Some((Pin { inner: inner.clone(), name: name.to_string(), version }, size))
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        self.inner.state().unpin(&self.name, self.version);
    }
}

pub struct Reader {
    _pin: Option<Pin>,
    file: Arc<File>,
    size: u64,
    position: u64,
    chunk_size: usize,
}

impl Reader {
    pub(super) fn new(pin: Pin, file: File, size: u64) -> Reader {
        let chunk_size = pin.inner.chunk_size;
        Reader { _pin: Some(pin), file: Arc::new(file), size, position: 0, chunk_size }
    }

    pub(super) fn writing(file: Arc<File>, chunk_size: usize) -> Reader {
        Reader { _pin: None, file, size: u64::MAX, position: 0, chunk_size }
    }

    pub async fn get_range(&self, start: u64, len: u64) -> Option<Vec<u8>> {
        let file = self.file.clone();
        let wanted = len.min(self.size.saturating_sub(start));
        tokio::task::spawn_blocking(move || super::byte_store::read_at(&file, start, wanted)).await.ok()?
    }

    pub fn len(&self) -> u64 {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub async fn read_chunk(&mut self) -> Result<Vec<u8>, CacheError> {
        let buffer = self.get_range(self.position, self.chunk_size as u64).await.ok_or_else(|| CacheError::Io("cache read failed".into()))?;
        self.position += buffer.len() as u64;
        Ok(buffer)
    }
}

#[async_trait::async_trait]
impl crate::proxy::StoreReader for Reader {
    fn version(&self) -> Option<(u64, u64)> {
        self._pin.as_ref().map(|pin| (pin.version, self.size))
    }

    async fn get_range(&mut self, start: u64, len: u64) -> Option<Vec<u8>> {
        Reader::get_range(self, start, len).await
    }
}
