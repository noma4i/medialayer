use super::memory_store::MemoryStore;
use super::test_client::get;
use super::*;
use crate::fetch::mock::{MockFetcher, Script};
use crate::memory::{Ledger, Owner};
use async_trait::async_trait;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use url::Url;

const KIB: u64 = 1024;
const STREAM_CHUNK: u64 = 64 * KIB;
const FLOOR: u64 = 4 * KIB;

#[derive(Default)]
struct Counters {
    peak_buffers: AtomicU64,
    appending: AtomicUsize,
    peak_appending: AtomicUsize,
    largest_read: AtomicU64,
}

#[derive(Clone)]
struct Probe {
    store: MemoryStore,
    ledger: Ledger,
    counters: Arc<Counters>,
}

impl Probe {
    fn new(store: &MemoryStore, ledger: &Ledger) -> Probe {
        Probe { store: store.clone(), ledger: ledger.clone(), counters: Arc::default() }
    }

    fn sample(&self) {
        self.counters.peak_buffers.fetch_max(self.ledger.used_by(Owner::ProxyBuffer), Ordering::SeqCst);
    }

    fn peak_buffers(&self) -> u64 {
        self.counters.peak_buffers.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ByteStore for Probe {
    async fn len(&self, key: &str) -> Option<u64> {
        self.store.len(key).await
    }

    async fn contains(&self, key: &str) -> bool {
        self.store.contains(key).await
    }

    async fn get_range(&self, key: &str, start: u64, len: u64) -> Option<Vec<u8>> {
        self.sample();
        self.counters.largest_read.fetch_max(len, Ordering::SeqCst);
        self.store.get_range(key, start, len).await
    }

    async fn meta(&self, key: &str) -> Option<Meta> {
        self.store.meta(key).await
    }

    async fn begin_write(&self, key: &str, meta: Meta) -> Option<Box<dyn StoreWriter>> {
        let writer = self.store.begin_write(key, meta).await?;
        Some(Box::new(ProbeWriter { writer, probe: self.clone() }))
    }

    async fn remove(&self, key: &str) {
        self.store.remove(key).await;
    }
}

struct ProbeWriter {
    writer: Box<dyn StoreWriter>,
    probe: Probe,
}

#[async_trait]
impl StoreWriter for ProbeWriter {
    async fn append(&mut self, bytes: &[u8]) -> bool {
        self.probe.sample();
        let counters = &self.probe.counters;
        let active = counters.appending.fetch_add(1, Ordering::SeqCst) + 1;
        counters.peak_appending.fetch_max(active, Ordering::SeqCst);
        tokio::task::yield_now().await;
        let appended = self.writer.append(bytes).await;
        counters.appending.fetch_sub(1, Ordering::SeqCst);
        appended
    }

    async fn commit(self: Box<Self>) -> bool {
        self.writer.commit().await
    }

    async fn commit_exact(self: Box<Self>, length: u64) -> bool {
        self.writer.commit_exact(length).await
    }

    async fn abort(self: Box<Self>) {
        self.writer.abort().await
    }
}

struct Rig {
    proxy: Proxy,
    fetcher: Arc<MockFetcher>,
    store: MemoryStore,
    probe: Probe,
    ledger: Ledger,
}

async fn rig(ceiling: u64) -> Rig {
    let ledger = Ledger::new(ceiling);
    let config = Config { ledger: ledger.clone(), ..Config::default() };
    let fetcher = Arc::new(MockFetcher::new());
    let store = MemoryStore::new();
    let probe = Probe::new(&store, &ledger);
    let proxy = Proxy::start(config, fetcher.clone(), Arc::new(probe.clone())).await.expect("proxy start");
    Rig { proxy, fetcher, store, probe, ledger }
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

fn target_of(proxy: &Proxy, url: &str) -> String {
    let routed = proxy.url_for(url, &HashMap::new()).unwrap();
    let parsed = Url::parse(&routed).unwrap();
    format!("{}?{}", parsed.path(), parsed.query().unwrap_or_default())
}

async fn until(mut condition: impl FnMut() -> bool) {
    for _ in 0..1_000_000 {
        if condition() {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("condition not reached");
}

#[tokio::test]
async fn inv_proxy_13_downloads_reserve_their_buffers_and_never_pull_more_than_the_ledger_allows() {
    let rig = rig(STREAM_CHUNK).await;
    let urls: Vec<String> = (0..3).map(|index| format!("https://o.test/v{index}.mp4")).collect();
    for url in &urls {
        rig.fetcher.route(url, Script::ok(pattern(200_000)).chunked(16 * KIB as usize));
    }

    let no_headers = HashMap::new();
    let loads = futures_util::future::join_all(urls.iter().map(|url| rig.proxy.fetch_into_store(url, &no_headers, Priority::Prefetch, None))).await;

    assert!(loads.iter().all(Result::is_ok));
    assert!(urls.iter().all(|url| rig.store.bytes(url) == Some(pattern(200_000))));
    assert!(rig.probe.peak_buffers() > 0, "buffers were never reserved");
    assert!(rig.probe.peak_buffers() <= STREAM_CHUNK);
    assert_eq!(rig.probe.counters.peak_appending.load(Ordering::SeqCst), 1);
    assert_eq!(rig.ledger.used(), 0);
}

#[tokio::test]
async fn inv_proxy_13_serving_from_the_store_shrinks_its_chunks_to_what_the_ledger_has_free() {
    let rig = rig(STREAM_CHUNK / 4).await;
    let url = "https://o.test/cached.mp4";
    rig.store.seed(url, &pattern(100_000));
    let _held = rig.ledger.reserve(Owner::Slot(0), STREAM_CHUNK / 4 - FLOOR).unwrap();

    let reply = get(rig.proxy.port(), &target_of(&rig.proxy, url)).await;

    assert_eq!(reply.body, pattern(100_000));
    assert!(rig.probe.counters.largest_read.load(Ordering::SeqCst) <= FLOOR);
    assert!(rig.probe.peak_buffers() > 0 && rig.probe.peak_buffers() <= FLOOR);
}

#[tokio::test]
async fn inv_proxy_13_serving_waits_while_the_ledger_is_full_and_continues_when_memory_is_freed() {
    let rig = rig(STREAM_CHUNK).await;
    let url = "https://o.test/cached.mp4";
    rig.store.seed(url, &pattern(100_000));
    let held = rig.ledger.reserve(Owner::Slot(0), STREAM_CHUNK).unwrap();
    let port = rig.proxy.port();
    let target = target_of(&rig.proxy, url);
    let client = tokio::spawn(async move { get(port, &target).await });

    until(|| rig.ledger.waiting() == 1).await;
    assert_eq!(rig.probe.counters.largest_read.load(Ordering::SeqCst), 0);
    drop(held);

    assert_eq!(client.await.unwrap().body, pattern(100_000));
    assert_eq!(rig.ledger.used(), 0);
}

const PLAYLIST: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\nseg0.ts\n#EXT-X-ENDLIST\n";

#[tokio::test]
async fn inv_proxy_13_a_stored_playlist_is_read_under_a_reservation_of_its_size() {
    let rig = rig(STREAM_CHUNK).await;
    let url = "https://o.test/list.m3u8";
    rig.store.seed(url, PLAYLIST.as_bytes());

    let reply = get(rig.proxy.port(), &target_of(&rig.proxy, url)).await;

    assert_eq!(reply.status, 200);
    assert_eq!(rig.probe.peak_buffers(), PLAYLIST.len() as u64);
}

#[tokio::test]
async fn inv_proxy_13_a_stored_object_is_read_for_prefetch_under_a_reservation_of_its_size() {
    let rig = rig(STREAM_CHUNK).await;
    let url = "https://o.test/list.m3u8";
    rig.store.seed(url, PLAYLIST.as_bytes());

    let bytes = rig.proxy.read_stored(url, &HashMap::new(), 1024).await;

    assert_eq!(bytes.as_deref(), Some(PLAYLIST.as_bytes()));
    assert_eq!(rig.probe.peak_buffers(), PLAYLIST.len() as u64);
}

#[tokio::test]
async fn inv_proxy_13_an_object_over_the_read_limit_is_not_read_into_memory() {
    let rig = rig(STREAM_CHUNK).await;
    let url = "https://o.test/list.m3u8";
    rig.store.seed(url, PLAYLIST.as_bytes());

    assert_eq!(rig.proxy.read_stored(url, &HashMap::new(), 8).await, None);
    assert_eq!(rig.probe.counters.largest_read.load(Ordering::SeqCst), 0);
}
