use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{stream, StreamExt};
use crate::cache::{classify_kind, Cache, CacheStore, Kind, SystemClock};
use crate::fetch::{Fetcher, FetchError, Request, Response};
use crate::limits::Limits;
use crate::prefetch::{CacheStrategy, Loader, LoadError, Prefetcher, PrefetchLedger, Source, SourceKind};
use crate::proxy::test_client::get as client_get;
use crate::proxy::{Config, Priority, Proxy};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::Notify;

fn limits(budget: u64) -> Limits {
    Limits { budget, min_budget: 1, ..Limits::default() }
}

async fn cache(budget: u64) -> (tempfile::TempDir, Cache) {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), limits(budget), u64::MAX);
    cache.ready().await;
    (dir, cache)
}

#[tokio::test]
async fn inv_cache_01_pending_writes_share_disk_budget() {
    let (dir, cache) = cache(100).await;
    let mut a = cache.begin_write("a", Kind::Stream).await.unwrap();
    let mut b = cache.begin_write("b", Kind::Stream).await.unwrap();
    a.write(&[0; 70]).await.unwrap();
    let _ = b.write(&[0; 70]).await;
    let bytes: u64 = std::fs::read_dir(dir.path()).unwrap().flatten()
        .filter(|f| f.file_name().to_string_lossy().ends_with(".part"))
        .map(|f| f.metadata().unwrap().len()).sum();
    assert!(bytes <= 100, "pending bytes={bytes}, budget=100");
}

struct Origin {
    requests: Mutex<Vec<Request>>,
    started: Notify,
    hang: bool,
    size: usize,
}

#[async_trait]
impl Fetcher for Origin {
    async fn fetch(&self, request: Request) -> Result<Response, FetchError> {
        let range = request.range;
        self.requests.lock().unwrap().push(request);
        self.started.notify_one();
        let body = if self.hang { stream::pending().boxed() } else {
            stream::iter((0..self.size).step_by(64).map(|i| Ok(Bytes::from(vec![7; 64.min(self.size-i)]))).collect::<Vec<_>>()).boxed()
        };
        Ok(Response {
            status: if range.is_some() {206} else {200},
            headers: HashMap::from([("content-length".into(), self.size.to_string()), ("content-range".into(), format!("bytes 0-{}/{}", self.size-1, self.size))]),
            body,
        })
    }
}

async fn proxy(cache: &Cache, hang: bool, size: usize) -> (Proxy, Arc<Origin>) {
    let origin = Arc::new(Origin { requests: Mutex::new(Vec::new()), started: Notify::new(), hang, size });
    let store = Arc::new(CacheStore::new(cache.clone(), Arc::new(classify_kind)));
    let proxy = Proxy::start(Config::default(), origin.clone(), store).await.unwrap();
    (proxy, origin)
}

async fn get(proxy: &Proxy, url: &str) -> Vec<u8> {
    let parsed = url::Url::parse(url).unwrap();
    client_get(proxy.port(), &format!("{}?{}", parsed.path(), parsed.query().unwrap())).await.body
}

#[tokio::test]
async fn inv_proxy_01_registered_headers_stay_with_their_source() {
    let (_dir, cache) = cache(10000).await;
    let (proxy, origin) = proxy(&cache, false, 10).await;
    let a = proxy.url_for("https://origin.test/a.mp4", &HashMap::from([("Authorization".into(), "token-A".into())])).unwrap();
    let _b = proxy.url_for("https://origin.test/b.mp4", &HashMap::from([("Authorization".into(), "token-B".into())])).unwrap();
    let _ = get(&proxy, &a).await;
    proxy.stop().await;
    assert_eq!(origin.requests.lock().unwrap()[0].headers.get("Authorization").map(String::as_str), Some("token-A"));
}

#[tokio::test]
async fn inv_proxy_02_media_larger_than_cache_still_plays() {
    let (_dir, cache) = cache(100).await;
    let (proxy, _origin) = proxy(&cache, false, 500).await;
    let url = proxy.url_for("https://origin.test/a.mp4", &HashMap::new()).unwrap();
    let bytes = get(&proxy, &url).await.len();
    proxy.stop().await;
    assert_eq!(bytes, 500, "received bytes={bytes}, origin bytes=500");
}

#[tokio::test]
async fn inv_proxy_06_cancel_first_block_stops_download() {
    let (_dir, cache) = cache(10000).await;
    let (proxy, origin) = proxy(&cache, true, 10).await;
    let running = proxy.clone();
    let task = tokio::spawn(async move { running.fetch_first_block("https://origin.test/a.mp4", &HashMap::new(), Priority::Prefetch).await });
    origin.started.notified().await;
    task.abort();
    let _ = task.await;
    crate::api::test_support::eventually(|| proxy.live_tasks() == 0).await;
    proxy.stop().await;
}

struct HangingLoader {
    calls: AtomicUsize,
    active: AtomicUsize,
    started: Notify,
}

struct Active<'a>(&'a HangingLoader);
impl Drop for Active<'_> {
    fn drop(&mut self) { self.0.active.fetch_sub(1, Ordering::SeqCst); }
}

#[async_trait]
impl Loader for HangingLoader {
    fn cache_key(&self, url: &str, headers: &HashMap<String, String>) -> String {
        crate::identity::namespaced_key(url, headers, &[])
    }

    async fn load(&self, _: &str, _: &HashMap<String,String>, _: Priority, _: &Notify) -> Result<u64, LoadError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.active.fetch_add(1, Ordering::SeqCst);
        let _active = Active(self);
        self.started.notify_one();
        std::future::pending().await
    }
    async fn load_first_block(&self, url: &str, headers: &HashMap<String,String>, priority: Priority) -> Result<u64, LoadError> {
        self.load(url, headers, priority, &Notify::new()).await
    }
    async fn load_all_blocks(&self, url: &str, headers: &HashMap<String,String>, priority: Priority, progress: &Notify) -> Result<u64, LoadError> {
        self.load(url, headers, priority, progress).await
    }
    async fn playlist(&self, _: &str, _: &HashMap<String,String>) -> Option<Arc<crate::proxy::StoredPlaylist>> { None }
    async fn stored_len(&self, _: &str, _: &HashMap<String,String>) -> Option<u64> { None }
    async fn evict(&self, _: &str, _: &HashMap<String,String>) {}
}

fn sources() -> Vec<Option<Source>> {
    (0..4).map(|i| Some(Source { url: format!("https://o.test/{i}.jpg"), headers: HashMap::new(), kind: SourceKind::Image, strategy: CacheStrategy::Window })).collect()
}

fn prefetch() -> (Prefetcher, Arc<HangingLoader>, Limits) {
    let loader = Arc::new(HangingLoader { calls: AtomicUsize::new(0), active: AtomicUsize::new(0), started: Notify::new() });
    let limits = Limits { max_concurrent: 1, prefetch_from: 1, prefetch_to: 1, timeout_seconds: 100, ..Limits::default() };
    let ledger = PrefetchLedger::new(Duration::from_secs(60), 100);
    (Prefetcher::new(loader.clone(), &limits, ledger, Arc::new(SystemClock)), loader, limits)
}

#[tokio::test]
async fn inv_prefetch_02_set_limits_preserves_total_concurrency() {
    let (prefetch, loader, mut limits) = prefetch();
    prefetch.set_window(0, sources());
    loader.started.notified().await;
    limits.prefetch_to = 2;
    prefetch.set_limits(&limits);
    prefetch.set_window(0, sources());
    for _ in 0..100 { tokio::task::yield_now().await; }
    let active = loader.active.load(Ordering::SeqCst);
    prefetch.stop_all();
    assert!(active <= 1, "active={active}, max_concurrent=1");
}

#[tokio::test]
async fn inv_prefetch_04_drop_prefetch_stops_owned_tasks() {
    let (prefetch, loader, _limits) = prefetch();
    prefetch.set_window(0, sources());
    loader.started.notified().await;
    drop(prefetch);
    crate::api::test_support::eventually(|| loader.active.load(Ordering::SeqCst) == 0).await;
}

#[tokio::test]
async fn inv_proxy_07_drop_proxy_closes_owned_listener() {
    let (_dir, cache) = cache(10000).await;
    let (proxy, _origin) = proxy(&cache, false, 10).await;
    let port = proxy.port();
    drop(proxy);
    crate::api::test_support::eventually_async(|| async { tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err() }).await;
}
