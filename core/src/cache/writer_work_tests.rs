use super::*;
use crate::api::MediaError;
use crate::ffi::{BodyReader, FetchRequest, FetchResponse, Fetcher, ForeignFetcher};
use crate::fetch::Fetcher as _;
use futures_util::StreamExt;
use std::sync::atomic::AtomicUsize;

struct Body {
    remaining: AtomicUsize,
    chunk: usize,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl BodyReader for Body {
    async fn next_chunk(&self) -> Result<Option<Vec<u8>>, MediaError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let remaining = self.remaining.load(Ordering::Relaxed);
        let size = remaining.min(self.chunk);
        self.remaining.fetch_sub(size, Ordering::Relaxed);
        Ok((size > 0).then(|| vec![23; size]))
    }
    fn release(&self) {}
}

struct Origin(Arc<Body>);

#[async_trait::async_trait]
impl Fetcher for Origin {
    async fn fetch(&self, _: FetchRequest) -> Result<FetchResponse, MediaError> {
        Ok(FetchResponse { status: 200, headers: Default::default(), body: self.0.clone() })
    }
}

#[tokio::test]
async fn inv_cache_25_chunks_flush_only_at_commit() {
    const BYTES: usize = 256 * 1024;
    for chunk in [1024, 64 * 1024] {
        let directory = tempfile::tempdir().unwrap();
        let cache = crate::cache::Cache::open(directory.path(), crate::limits::Limits::default(), u64::MAX);
        cache.ready().await;
        let body = Arc::new(Body { remaining: AtomicUsize::new(BYTES), chunk, calls: AtomicUsize::new(0) });
        let fetcher = ForeignFetcher::new(Arc::new(Origin(body.clone())), crate::memory::InputLimits::default());
        let mut fetched = fetcher.fetch(crate::fetch::Request { url: "https://chunk.test/video.mp4".into(), headers: Default::default(), range: None }).await.unwrap();
        let mut writer = cache.begin_write("video", Kind::Stream).await.unwrap();
        let counts = writer.io_calls.clone();
        while let Some(bytes) = fetched.body.next().await { writer.write(&bytes.unwrap()).await.unwrap(); }
        writer.commit().await.unwrap();
        let calls = body.calls.load(Ordering::Relaxed);
        let writes = counts.0.load(Ordering::Relaxed);
        let flushes = counts.1.load(Ordering::Relaxed);
        eprintln!("chunk={chunk} bytes={BYTES} ffi_next_chunk={calls} write_all={writes} flush={flushes}");
        eprintln!("copied_bytes={} reservation_checks={}", cache.inner.copied_bytes.load(Ordering::Relaxed), cache.inner.reservation_checks.load(Ordering::Relaxed));
        assert_eq!(cache.get("video").await.unwrap(), vec![23; BYTES]);
        assert_eq!((calls, writes, flushes), (BYTES / chunk + 1, BYTES / chunk, 1));
        assert_eq!(cache.inner.reservation_checks.load(Ordering::Relaxed), 1);
    }
}
