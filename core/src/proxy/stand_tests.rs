use super::route::block_key;
use super::test_client::{open, read_at_least, request, try_request};
use super::*;
use crate::cache::{classify_kind, Cache, CacheStore, Options};
use crate::fetch::mock::{MockFetcher, Script};
use crate::fetch::{BodyStream, FetchError, Fetcher, Response};
use crate::limits::{EvictionStrategy, Limits};
use crate::memory::Ledger;
use crate::seeded::Dice;
use async_trait::async_trait;
use futures_util::StreamExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use url::Url;

const BLOCK: u64 = 16 * 1024;
const LANES: u64 = 8;
const REQUESTS: usize = 14;
const OBJECTS: usize = 5;
const MAX_CONCURRENT: usize = 3;

struct Truth {
    url: String,
    body: Vec<u8>,
    cut_at: Option<u64>,
}

struct CountingFetcher {
    inner: MockFetcher,
    active: AtomicUsize,
    peak: AtomicUsize,
}

#[async_trait]
impl Fetcher for Arc<CountingFetcher> {
    async fn fetch(&self, request: crate::fetch::Request) -> Result<Response, FetchError> {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        let guard = ActiveOwned(self.clone());
        let response = self.inner.fetch(request).await?;
        let Response { status, headers, body } = response;
        let body: BodyStream = Box::pin(futures_util::stream::unfold((body, Some(guard)), |(mut body, guard)| async move {
            body.next().await.map(|chunk| (chunk, (body, guard)))
        }));
        Ok(Response { status, headers, body })
    }
}

struct ActiveOwned(Arc<CountingFetcher>);

impl Drop for ActiveOwned {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

struct FlakyStore {
    inner: Arc<dyn ByteStore>,
    dice: Mutex<Dice>,
    failures: AtomicUsize,
}

impl FlakyStore {
    fn roll(&self, percent: u64) -> bool {
        let hit = self.dice.lock().unwrap().percent(percent);
        if hit {
            self.failures.fetch_add(1, Ordering::SeqCst);
        }
        hit
    }
}

struct FlakyWriter {
    inner: Box<dyn StoreWriter>,
    store: Arc<FlakyStore>,
}

#[async_trait]
impl ByteStore for Arc<FlakyStore> {
    async fn len(&self, key: &str) -> Option<u64> {
        self.inner.len(key).await
    }

    async fn contains(&self, key: &str) -> bool {
        self.inner.contains(key).await
    }

    async fn get_range(&self, key: &str, start: u64, len: u64) -> Option<Vec<u8>> {
        if self.roll(3) {
            return None;
        }
        self.inner.get_range(key, start, len).await
    }

    async fn meta(&self, key: &str) -> Option<Meta> {
        self.inner.meta(key).await
    }

    async fn begin_write(&self, key: &str, meta: Meta) -> Option<Box<dyn StoreWriter>> {
        if self.roll(4) {
            return None;
        }
        let inner = self.inner.begin_write(key, meta).await?;
        Some(Box::new(FlakyWriter { inner, store: self.clone() }))
    }

    async fn remove(&self, key: &str) {
        self.inner.remove(key).await;
    }
}

#[async_trait]
impl StoreWriter for FlakyWriter {
    async fn append(&mut self, bytes: &[u8]) -> bool {
        if self.store.roll(2) {
            return false;
        }
        self.inner.append(bytes).await
    }

    async fn commit(self: Box<Self>) -> bool {
        if self.store.roll(4) {
            return false;
        }
        self.inner.commit().await
    }

    async fn commit_exact(self: Box<Self>, length: u64) -> bool {
        if self.store.roll(4) {
            return false;
        }
        self.inner.commit_exact(length).await
    }

    async fn abort(self: Box<Self>) {
        self.inner.abort().await;
    }
}

fn path_of(proxy_url: &str) -> String {
    let parsed = Url::parse(proxy_url).unwrap();
    format!("{}?{}", parsed.path(), parsed.query().unwrap_or_default())
}

fn truth_set(dice: &mut Dice) -> Vec<Truth> {
    (0..OBJECTS)
        .map(|index| {
            let len = dice.between(8_000, 90_000) as usize;
            let body = dice.bytes(len);
            let cut_at = (index == OBJECTS - 1).then(|| dice.between(1_000, len as u64 - 3_000));
            Truth { url: format!("https://o.test/stand/{index}.mp4"), body, cut_at }
        })
        .collect()
}

fn range_header(dice: &mut Dice, total: u64) -> (Option<String>, Option<(u64, u64)>) {
    match dice.below(5) {
        0 => (None, Some((0, total - 1))),
        1 => {
            let start = dice.below(total + 4);
            (Some(format!("bytes={start}-")), (start < total).then_some((start, total - 1)))
        }
        2 => {
            let start = dice.below(total);
            let end = start + dice.below(total);
            (Some(format!("bytes={start}-{end}")), Some((start, end.min(total - 1))))
        }
        3 => {
            let length = dice.between(1, total + 10);
            (Some(format!("bytes=-{length}")), Some((total.saturating_sub(length), total - 1)))
        }
        _ => {
            let start = dice.below(total);
            (Some(format!("bytes={start}-{start}")), Some((start, start)))
        }
    }
}

async fn lane(proxy: Proxy, truths: Arc<Vec<Truth>>, mut dice: Dice, hurt: Arc<AtomicUsize>) {
    for _ in 0..REQUESTS {
        let truth = &truths[dice.index(truths.len())];
        let total = truth.body.len() as u64;
        let target = path_of(&proxy.url_for(&truth.url, &HashMap::new()).expect("proxy url"));
        let (header, expected) = range_header(&mut dice, total);
        let headers: Vec<(&str, &str)> = header.iter().map(|value| ("Range", value.as_str())).collect();
        if dice.percent(15) {
            let mut stream = open(proxy.port(), "GET", &target, &headers).await;
            read_at_least(&mut stream, dice.between(1, 3_000) as usize).await;
            drop(stream);
            continue;
        }
        let reply = if truth.cut_at.is_some() {
            match try_request(proxy.port(), "GET", &target, &headers).await {
                Some(reply) => reply,
                None => continue,
            }
        } else {
            request(proxy.port(), "GET", &target, &headers).await
        };
        let Some((start, end)) = expected else {
            assert_eq!(reply.status, 416, "range {header:?} of {total} bytes is unsatisfiable");
            continue;
        };
        let wanted = &truth.body[start as usize..=end as usize];
        if truth.cut_at.is_some() {
            if reply.status < 300 {
                hurt.fetch_add(1, Ordering::SeqCst);
                assert!(wanted.starts_with(&reply.body), "{header:?} of the cut object returned bytes that are not a prefix of the truth");
            }
            continue;
        }
        assert_eq!(reply.status, if header.is_some() { 206 } else { 200 }, "status for {header:?} of {}", truth.url);
        assert_eq!(reply.body.len(), wanted.len(), "{header:?} of {}: length", truth.url);
        assert!(reply.body == wanted, "{header:?} of {}: the bytes differ from the origin", truth.url);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inv_proxy_02_inv_proxy_03_inv_proxy_06_inv_proxy_11_clients_ranges_cuts_and_store_failures_always_get_the_origin_bytes_and_leave_nothing_behind() {
    let name = "inv_proxy_02_inv_proxy_03_inv_proxy_06_inv_proxy_11_clients_ranges_cuts_and_store_failures_always_get_the_origin_bytes_and_leave_nothing_behind";
    let mut dice = Dice::new(name);
    let truths = Arc::new(truth_set(&mut dice));
    let mock = MockFetcher::new();
    for truth in truths.iter() {
        let chunk = if truth.cut_at.is_some() { 700 } else { dice.between(500, 5_000) as usize };
        let mut script = Script::ok(truth.body.clone()).header("Content-Type", "video/mp4").chunked(chunk);
        script = script.chunk_delay(std::time::Duration::from_micros(dice.between(0, 300)));
        if let Some(cut) = truth.cut_at {
            script = script.cut_after(cut);
        }
        mock.route(&truth.url, script);
    }
    let fetcher = Arc::new(CountingFetcher { inner: mock, active: AtomicUsize::new(0), peak: AtomicUsize::new(0) });

    let dir = tempfile::tempdir().unwrap();
    let limits = Limits { budget: 120_000, min_budget: 1, shrink_to: 0.5, strategy: EvictionStrategy::Lru, ..Limits::default() };
    let cache = Cache::open_with(dir.path(), limits, Options { persist_delay: std::time::Duration::from_millis(20), ..Options::default() });
    cache.ready().await;
    let store = Arc::new(FlakyStore {
        inner: Arc::new(CacheStore::new(cache.clone(), Arc::new(classify_kind))),
        dice: Mutex::new(dice.fork(1_000)),
        failures: AtomicUsize::new(0),
    });
    let ledger = Ledger::new(256 * 1024);
    let mut config = Config::default();
    config.max_concurrent = MAX_CONCURRENT;
    config.chunk_size = BLOCK;
    config.stream_chunk = 4 * 1024;
    config.ledger = ledger.clone();
    config.policy = config.policy.with_timeout(std::time::Duration::from_secs(5));
    let proxy = Proxy::start(config, Arc::new(fetcher.clone()), Arc::new(store.clone())).await.expect("proxy start");

    let hurt = Arc::new(AtomicUsize::new(0));
    let lanes: Vec<_> = (0..LANES).map(|index| tokio::spawn(lane(proxy.clone(), truths.clone(), dice.fork(index), hurt.clone()))).collect();
    for lane in lanes {
        tokio::time::timeout(std::time::Duration::from_secs(60), lane).await.expect("a client lane hung").unwrap();
    }

    assert!(store.failures.load(Ordering::SeqCst) > 0, "the store injected too few failures: {}", store.failures.load(Ordering::SeqCst));
    assert!(fetcher.peak.load(Ordering::SeqCst) <= MAX_CONCURRENT, "{} origin responses were open at once", fetcher.peak.load(Ordering::SeqCst));
    let quiet = async {
        loop {
            let stats = cache.stats().await;
            if proxy.live_tasks() == 0
                && proxy.inner.slots.in_use() == 0
                && ledger.used() == 0
                && stats.open_writers == 0
                && fetcher.active.load(Ordering::SeqCst) == 0
                && std::fs::read_dir(dir.path()).unwrap().flatten().all(|entry| !entry.file_name().to_string_lossy().ends_with(".part"))
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(15), quiet).await.unwrap_or_else(|_| {
        panic!(
            "leftovers: tasks {} slots {} ledger {} fetches {}",
            proxy.live_tasks(),
            proxy.inner.slots.in_use(),
            ledger.used(),
            fetcher.active.load(Ordering::SeqCst)
        )
    });

    for truth in truths.iter() {
        let key = proxy.key_for(&truth.url, &HashMap::new());
        if let Some(stored) = cache.get(&key).await {
            assert!(truth.cut_at.is_none(), "{} was cut at the origin but a whole entry was stored", truth.url);
            assert!(stored == truth.body, "{}: the stored entry differs from the origin", truth.url);
        }
        for block in 0..=(truth.body.len() as u64 / BLOCK) {
            let Some(stored) = cache.get(&block_key(&key, block)).await else { continue };
            let from = (block * BLOCK) as usize;
            let to = (from + BLOCK as usize).min(truth.body.len());
            assert!(stored == truth.body[from..to], "{} block {block}: the stored block differs from the origin", truth.url);
        }
    }
}

struct SlotProbe {
    inner: MockFetcher,
    slots: Arc<std::sync::OnceLock<super::slots::PrioritySlots>>,
    held_at_close: Arc<Mutex<Vec<usize>>>,
}

struct CloseWatch {
    slots: Arc<std::sync::OnceLock<super::slots::PrioritySlots>>,
    held_at_close: Arc<Mutex<Vec<usize>>>,
}

impl Drop for CloseWatch {
    fn drop(&mut self) {
        let held = self.slots.get().map_or(0, |slots| slots.in_use());
        self.held_at_close.lock().unwrap().push(held);
    }
}

#[async_trait]
impl Fetcher for SlotProbe {
    async fn fetch(&self, request: crate::fetch::Request) -> Result<Response, FetchError> {
        let Response { status, headers, body } = self.inner.fetch(request).await?;
        let watch = CloseWatch { slots: self.slots.clone(), held_at_close: self.held_at_close.clone() };
        let body: BodyStream = Box::pin(futures_util::stream::unfold((body, watch), |(mut body, watch)| async move {
            body.next().await.map(|chunk| (chunk, (body, watch)))
        }));
        Ok(Response { status, headers, body })
    }
}

#[tokio::test]
async fn inv_proxy_10_an_origin_body_is_closed_while_its_slot_is_still_held() {
    let mock = MockFetcher::new();
    mock.route("https://o.test/probe.mp4", Script::ok(vec![7u8; 3_000]).header("Content-Type", "video/mp4"));
    let slots = Arc::new(std::sync::OnceLock::new());
    let held_at_close = Arc::new(Mutex::new(Vec::new()));
    let fetcher = Arc::new(SlotProbe { inner: mock, slots: slots.clone(), held_at_close: held_at_close.clone() });
    let config = Config { max_concurrent: 1, ..Config::default() };
    let proxy = Proxy::start(config, fetcher, Arc::new(super::memory_store::MemoryStore::new())).await.unwrap();
    slots.set(proxy.inner.slots.clone()).ok().unwrap();
    let target = path_of(&proxy.url_for("https://o.test/probe.mp4", &HashMap::new()).unwrap());
    let head = request(proxy.port(), "HEAD", &target, &[]).await;
    assert_eq!(head.status, 200);
    let closes = held_at_close.lock().unwrap().clone();
    assert!(!closes.is_empty(), "the origin body was never closed");
    assert!(closes.iter().all(|held| *held == 1), "an origin body outlived its slot: slots held at each close {closes:?}");
}

#[tokio::test]
async fn inv_proxy_06_a_download_whose_last_client_just_left_is_not_cancelled_under_a_client_that_joined_it() {
    let mock = MockFetcher::new();
    mock.route("https://o.test/join.mp4", Script::ok(vec![5u8; 4_000]).header("Content-Type", "video/mp4").chunked(500));
    let proxy = Proxy::start(Config::default(), Arc::new(mock), Arc::new(super::memory_store::MemoryStore::new())).await.unwrap();
    let url = "https://o.test/join.mp4";
    let first = proxy.inner.get_or_start(url, HashMap::new(), Priority::Playback);
    drop(first);
    let mut second = proxy.inner.existing_flight(url, &HashMap::new()).expect("the download is still joinable");
    let progress = tokio::time::timeout(std::time::Duration::from_secs(5), second.wait_finished(None)).await.expect("download hung");
    assert!(matches!(progress.outcome, super::flight::FlightOutcome::Complete), "the joined download ended as {:?}", progress.outcome);
    assert_eq!(progress.written, 4_000);
}
