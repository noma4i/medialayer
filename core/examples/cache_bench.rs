use medialayer::cache::{classify_kind, Cache, CacheStore, Options};
use medialayer::fetch::{FetchError, Fetcher, Request, Response};
use medialayer::limits::Limits;
use medialayer::proxy::{ByteStore, Config, Meta, Proxy};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const BLOCK: u64 = MIB;
const APPEND: usize = 64 * 1024;
const MIN_FREE_GIB: u64 = 50;

// Size percentiles (p10, p50, p90) of the zooko development blobs, measured with SQL on active_storage_blobs.
const IMAGE_SIZES: [u64; 3] = [4_000, 14_000, 200_000];
const VIDEO_SIZES: [u64; 3] = [800_000, 4_000_000, 24_000_000];
const VIDEO_BYTE_SHARE: f64 = 0.65;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn sized(&mut self, points: [u64; 3]) -> u64 {
        let (low, mid, high) = (points[0] as f64, points[1] as f64, points[2] as f64);
        let unit = self.unit();
        let value = if unit < 0.5 { low * (mid / low).powf(unit / 0.4) } else { mid * (high / mid).powf((unit - 0.5) / 0.4) };
        value.clamp(low / 4.0, high * 2.0) as u64
    }
}

fn fill_bytes(buffer: &mut [u8], seed: u64) {
    let mut rng = Rng(seed | 1);
    for word in buffer.chunks_mut(8) {
        let value = rng.next().to_le_bytes();
        word.copy_from_slice(&value[..word.len()]);
    }
}

fn seed_of(key: &str) -> u64 {
    key.bytes().fold(0xcbf29ce484222325u64, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3))
}

struct Slow<S> {
    inner: S,
    delay: Duration,
}

#[async_trait::async_trait]
impl<S: ByteStore> ByteStore for Slow<S> {
    async fn open_reader(&self, key: &str) -> Option<Box<dyn medialayer::proxy::StoreReader>> {
        Some(Box::new(SlowReader { inner: self.inner.open_reader(key).await?, delay: self.delay }))
    }
    async fn len(&self, key: &str) -> Option<u64> {
        self.inner.len(key).await
    }
    async fn contains(&self, key: &str) -> bool {
        self.inner.contains(key).await
    }
    async fn get_range(&self, key: &str, start: u64, len: u64) -> Option<Vec<u8>> {
        tokio::time::sleep(self.delay).await;
        self.inner.get_range(key, start, len).await
    }
    async fn meta(&self, key: &str) -> Option<Meta> {
        self.inner.meta(key).await
    }
    async fn begin_write(&self, key: &str, meta: Meta) -> Option<Box<dyn medialayer::proxy::StoreWriter>> {
        self.inner.begin_write(key, meta).await
    }
    async fn remove(&self, key: &str) {
        self.inner.remove(key).await
    }
}

struct SlowReader {
    inner: Box<dyn medialayer::proxy::StoreReader>,
    delay: Duration,
}

#[async_trait::async_trait]
impl medialayer::proxy::StoreReader for SlowReader {
    fn version(&self) -> Option<(u64, u64)> { self.inner.version() }
    async fn get_range(&mut self, start: u64, len: u64) -> Option<Vec<u8>> {
        tokio::time::sleep(self.delay).await;
        self.inner.get_range(start, len).await
    }
}

struct CountingFetcher(AtomicU64);

#[async_trait::async_trait]
impl Fetcher for CountingFetcher {
    async fn fetch(&self, _request: Request) -> Result<Response, FetchError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(FetchError::Network("bench has no network".to_string()))
    }
}

struct Args(HashMap<String, String>);

impl Args {
    fn parse() -> (String, Args) {
        let mut all = std::env::args().skip(1);
        let mode = all.next().unwrap_or_default();
        let mut map = HashMap::new();
        while let Some(name) = all.next() {
            map.insert(name.trim_start_matches("--").to_string(), all.next().unwrap_or_default());
        }
        (mode, Args(map))
    }

    fn text(&self, name: &str) -> String {
        self.0.get(name).cloned().unwrap_or_else(|| panic!("missing --{name}"))
    }

    fn number<T: std::str::FromStr>(&self, name: &str, default: T) -> T {
        self.0.get(name).and_then(|value| value.parse().ok()).unwrap_or(default)
    }
}

fn free_gib() -> u64 {
    let output = std::process::Command::new("df").args(["-g", "/System/Volumes/Data"]).output().expect("df");
    String::from_utf8_lossy(&output.stdout).lines().nth(1).and_then(|line| line.split_whitespace().nth(3)).and_then(|value| value.parse().ok()).unwrap_or(0)
}

fn manifest_path(dir: &Path) -> PathBuf {
    dir.with_extension("manifest.tsv")
}

async fn open_cache(dir: &Path, budget: u64) -> (Cache, Duration) {
    let limits = Limits { budget, min_budget: MIB, ..Limits::default() };
    let started = Instant::now();
    let cache = Cache::open_with(dir, limits, Options::default(), free_gib() * GIB);
    cache.ready().await;
    (cache, started.elapsed())
}

async fn write_entry(store: &CacheStore, key: &str, meta: Meta, length: u64, buffer: &mut Vec<u8>) {
    let mut writer = store.begin_write(key, meta).await.expect("writer");
    let mut seed = seed_of(key);
    let mut left = length;
    while left > 0 {
        let step = left.min(APPEND as u64) as usize;
        buffer.resize(step, 0);
        fill_bytes(buffer, seed);
        seed = seed.wrapping_add(1);
        assert!(writer.append(buffer).await, "append failed");
        left -= step as u64;
    }
    assert!(writer.commit().await, "commit failed");
}

async fn fill(args: &Args) {
    let dir = PathBuf::from(args.text("dir"));
    let target = (args.number("gib", 1.0f64) * GIB as f64) as u64;
    assert!(free_gib() > MIN_FREE_GIB + target / GIB, "not enough free space");
    let (cache, _) = open_cache(&dir, 1024 * GIB).await;
    let store = CacheStore::new(cache.clone(), Arc::new(classify_kind));
    let mut rng = Rng(args.number("seed", 7u64));
    let mut manifest = std::fs::File::create(manifest_path(&dir)).expect("manifest");
    let (mut video_bytes, mut total, mut files, mut buffer) = (0u64, 0u64, 0u64, Vec::new());
    let started = Instant::now();
    while total < target {
        let id = rng.next() % 1_000_000_000;
        if (video_bytes as f64) < VIDEO_BYTE_SHARE * (total as f64) + 1.0 {
            let length = rng.sized(VIDEO_SIZES).min(96 * MIB);
            let url = format!("https://cdn.example.com/videos/{id}.mp4");
            let meta = Meta { content_type: Some("video/mp4".to_string()), total_length: Some(length) };
            let mut index = 0;
            while index * BLOCK < length {
                let part = (length - index * BLOCK).min(BLOCK);
                let key = format!("{url}#{index}");
                write_entry(&store, &key, meta.clone(), part, &mut buffer).await;
                writeln!(manifest, "block\t{url}\t{index}\t{part}\t{length}").unwrap();
                index += 1;
                files += 1;
            }
            video_bytes += length;
            total += length;
        } else {
            let length = rng.sized(IMAGE_SIZES);
            let key = format!("https://cdn.example.com/images/{id}.jpg");
            let meta = Meta { content_type: Some("image/jpeg".to_string()), total_length: None };
            write_entry(&store, &key, meta, length, &mut buffer).await;
            writeln!(manifest, "image\t{key}\t0\t{length}\t{length}").unwrap();
            total += length;
            files += 1;
        }
    }
    cache.flush().await.unwrap();
    println!("FILL dir={} bytes={total} files={files} video_share={:.2} took={:.1}s", dir.display(), video_bytes as f64 / total as f64, started.elapsed().as_secs_f64());
}

struct Item {
    key: String,
    url: String,
    index: u64,
    length: u64,
    total: u64,
}

fn load_manifest(dir: &Path, kind: &str) -> Vec<Item> {
    let text = std::fs::read_to_string(manifest_path(dir)).expect("manifest");
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            (fields[0] == kind).then(|| {
                let (url, index) = (fields[1].to_string(), fields[2].parse().unwrap());
                let key = if kind == "block" { format!("{url}#{index}") } else { url.clone() };
                Item { key, url, index, length: fields[3].parse().unwrap(), total: fields[4].parse().unwrap() }
            })
        })
        .collect()
}

fn stats(samples: &mut [f64]) -> (f64, f64, f64) {
    samples.sort_by(|left, right| left.partial_cmp(right).unwrap());
    (samples[0], samples[samples.len() / 2], samples[samples.len() - 1])
}

fn summarize(label: &str, values: &mut [f64], unit: &str) {
    let (min, median, max) = stats(values);
    println!("SUMMARY {label}: min={min:.2}{unit} median={median:.2}{unit} max={max:.2}{unit} repeats={}", values.len());
}

async fn churn(store: CacheStore, stop: Arc<std::sync::atomic::AtomicBool>, rate_per_second: u64) {
    let mut buffer = Vec::new();
    let mut sequence = 0u64;
    while !stop.load(Ordering::SeqCst) {
        let key = format!("https://cdn.example.com/churn/{sequence}.mp4#0");
        let meta = Meta { content_type: Some("video/mp4".to_string()), total_length: Some(BLOCK) };
        write_entry(&store, &key, meta, BLOCK, &mut buffer).await;
        sequence += 1;
        tokio::time::sleep(Duration::from_millis(1000 / rate_per_second.max(1))).await;
    }
}

async fn read_cache(args: &Args) {
    let dir = PathBuf::from(args.text("dir"));
    let budget = (args.number("budget-gib", 4.0f64) * GIB as f64) as u64;
    let repeats = args.number("repeats", 5usize);
    let samples = args.number("samples", 200usize);
    let slow_ms = args.number("slow-ms", 0u64);
    let churn_rate = args.number("churn", 0u64);
    let items = load_manifest(&dir, "block");
    let (mut ready, mut first, mut whole, mut flush, mut again) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for repeat in 0..repeats {
        let (cache, took) = open_cache(&dir, budget).await;
        let plain = CacheStore::new(cache.clone(), Arc::new(classify_kind));
        let store: Arc<dyn ByteStore> = if slow_ms > 0 { Arc::new(Slow { inner: plain.clone(), delay: Duration::from_millis(slow_ms) }) } else { Arc::new(plain.clone()) };
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = (churn_rate > 0).then(|| tokio::spawn(churn(plain.clone(), stop.clone(), churn_rate)));
        let mut rng = Rng(1000 + repeat as u64);
        let picks: Vec<usize> = (0..samples).map(|_| (rng.next() % items.len() as u64) as usize).collect();
        let (mut firsts, mut wholes, mut missing) = (Vec::new(), Vec::new(), 0);
        let (mut again_firsts, mut again_wholes) = (Vec::new(), Vec::new());
        for pass in 0..2 {
            for pick in &picks {
                let item = &items[*pick];
                let started = Instant::now();
                let Some(head) = store.get_range(&item.key, 0, APPEND as u64).await else {
                    missing += 1;
                    continue;
                };
                let first_us = started.elapsed().as_secs_f64() * 1e6;
                let mut position = head.len() as u64;
                while position < item.length {
                    let step = (item.length - position).min(APPEND as u64);
                    position += store.get_range(&item.key, position, step).await.map_or(item.length, |bytes| bytes.len() as u64);
                }
                let whole_us = started.elapsed().as_secs_f64() * 1e6;
                if pass == 0 {
                    firsts.push(first_us);
                    wholes.push(whole_us);
                } else {
                    again_firsts.push(first_us);
                    again_wholes.push(whole_us);
                }
            }
        }
        stop.store(true, Ordering::SeqCst);
        if let Some(writer) = writer {
            let _ = writer.await;
        }
        let flushed = Instant::now();
        cache.flush().await.unwrap();
        let stats = cache.stats().await;
        let (_, first_median, _) = stats_of(&mut firsts);
        let (_, whole_median, _) = stats_of(&mut wholes);
        let (_, again_first_median, _) = stats_of(&mut again_firsts);
        let (_, again_whole_median, _) = stats_of(&mut again_wholes);
        println!(
            "RESULT repeat={repeat} entries={} bytes={} ready_ms={:.1} first_us_p50={first_median:.0} whole_us_p50={whole_median:.0} again_first_us_p50={again_first_median:.0} again_whole_us_p50={again_whole_median:.0} flush_ms={:.1} missing={missing} evictions={}",
            stats.file_count,
            stats.total_bytes,
            took.as_secs_f64() * 1e3,
            flushed.elapsed().as_secs_f64() * 1e3,
            stats.evictions
        );
        ready.push(took.as_secs_f64() * 1e3);
        first.push(first_median);
        whole.push(whole_median);
        again.push(again_whole_median);
        flush.push(flushed.elapsed().as_secs_f64() * 1e3);
        drop(store);
        drop(plain);
        drop(cache);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    summarize("ready", &mut ready, "ms");
    summarize("first_byte_p50", &mut first, "us");
    summarize("whole_block_p50", &mut whole, "us");
    summarize("whole_block_second_pass_p50", &mut again, "us");
    summarize("index_flush", &mut flush, "ms");
}

fn stats_of(samples: &mut [f64]) -> (f64, f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    stats(samples)
}

async fn touch_all(args: &Args) {
    let dir = PathBuf::from(args.text("dir"));
    let (cache, took) = open_cache(&dir, 64 * GIB).await;
    let store = CacheStore::new(cache.clone(), Arc::new(classify_kind));
    let (mut total, mut bytes, mut gone) = (0u64, 0u64, 0u64);
    let started = Instant::now();
    for kind in ["block", "image"] {
        for item in load_manifest(&dir, kind) {
            total += 1;
            match store.len(&item.key).await {
                Some(length) => bytes += length,
                None => gone += 1,
            }
        }
    }
    let touched = started.elapsed();
    let flushed = Instant::now();
    cache.flush().await.unwrap();
    println!(
        "RESULT ready_ms={:.1} entries={total} gone={gone} gib={:.2} touch_all_ms={:.0} touch_ms_per_gib={:.0} flush_ms={:.1}",
        took.as_secs_f64() * 1e3,
        bytes as f64 / GIB as f64,
        touched.as_secs_f64() * 1e3,
        touched.as_secs_f64() * 1e3 / (bytes as f64 / GIB as f64),
        flushed.elapsed().as_secs_f64() * 1e3
    );
}

async fn http_range(port: u16, target: &str, start: u64, end: u64) -> Option<(f64, f64, u64, u16)> {
    let started = Instant::now();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
    let request = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nRange: bytes={start}-{end}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.ok()?;
    let mut raw = Vec::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let (mut body_start, mut first) = (None, 0.0);
    loop {
        let read = stream.read(&mut buffer).await.ok()?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..read]);
        if body_start.is_none() {
            body_start = raw.windows(4).position(|window| window == b"\r\n\r\n").map(|at| at + 4);
        }
        if body_start.is_some_and(|at| raw.len() > at) && first == 0.0 {
            first = started.elapsed().as_secs_f64() * 1e6;
        }
    }
    let body = body_start.map_or(0, |at| (raw.len() - at) as u64);
    let status = String::from_utf8_lossy(&raw[..raw.len().min(16)]).split(' ').nth(1).and_then(|code| code.parse().ok()).unwrap_or(0);
    Some((first, started.elapsed().as_secs_f64() * 1e6, body, status))
}

async fn read_proxy(args: &Args) {
    let dir = PathBuf::from(args.text("dir"));
    let budget = (args.number("budget-gib", 4.0f64) * GIB as f64) as u64;
    let repeats = args.number("repeats", 5usize);
    let samples = args.number("samples", 60usize);
    let slow_ms = args.number("slow-ms", 0u64);
    let churn_rate = args.number("churn", 0u64);
    let mut videos: HashMap<String, Item> = HashMap::new();
    for item in load_manifest(&dir, "block") {
        if item.index == 0 {
            videos.insert(item.url.clone(), item);
        }
    }
    let videos: Vec<Item> = videos.into_values().collect();
    let (mut ready, mut first, mut whole, mut network_total) = (Vec::new(), Vec::new(), Vec::new(), 0u64);
    for repeat in 0..repeats {
        let (cache, took) = open_cache(&dir, budget).await;
        let plain = CacheStore::new(cache.clone(), Arc::new(classify_kind));
        let store: Arc<dyn ByteStore> = if slow_ms > 0 { Arc::new(Slow { inner: plain.clone(), delay: Duration::from_millis(slow_ms) }) } else { Arc::new(plain.clone()) };
        let fetcher = Arc::new(CountingFetcher(AtomicU64::new(0)));
        let config = Config { chunk_size: BLOCK, ..Config::default() };
        let proxy = Proxy::start(config, fetcher.clone(), store).await.expect("proxy");
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = (churn_rate > 0).then(|| tokio::spawn(churn(plain.clone(), stop.clone(), churn_rate)));
        let mut rng = Rng(5000 + repeat as u64);
        let (mut firsts, mut wholes, mut short) = (Vec::new(), Vec::new(), 0);
        for _ in 0..samples {
            let video = &videos[(rng.next() % videos.len() as u64) as usize];
            let Some(target) = proxy.url_for(&video.url, &HashMap::new()) else { panic!("proxy not ready") };
            let end = (video.total - 1).min(BLOCK * 2 - 1);
            let Some((first_us, whole_us, body, status)) = http_range(proxy.port(), &target, 0, end).await else { continue };
            if status != 206 || body != end + 1 {
                short += 1;
                continue;
            }
            firsts.push(first_us);
            wholes.push(whole_us);
        }
        stop.store(true, Ordering::SeqCst);
        if let Some(writer) = writer {
            let _ = writer.await;
        }
        let network = fetcher.0.load(Ordering::SeqCst);
        network_total += network;
        let (_, first_median, _) = stats_of(&mut firsts);
        let (_, whole_median, _) = stats_of(&mut wholes);
        println!("RESULT repeat={repeat} ready_ms={:.1} first_us_p50={first_median:.0} whole_us_p50={whole_median:.0} not_served={short} origin_fetches={network}", took.as_secs_f64() * 1e3);
        ready.push(took.as_secs_f64() * 1e3);
        first.push(first_median);
        whole.push(whole_median);
        proxy.stop().await;
        drop(proxy);
        drop(plain);
        drop(cache);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    summarize("proxy_ready", &mut ready, "ms");
    summarize("proxy_first_byte_p50", &mut first, "us");
    summarize("proxy_2mib_range_p50", &mut whole, "us");
    println!("SUMMARY origin_fetches_total={network_total}");
}

fn main() {
    let (mode, args) = Args::parse();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(args.number("workers", 4usize))
        .max_blocking_threads(args.number("blocking", 512usize))
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(run(mode, args));
}

async fn run(mode: String, args: Args) {
    match mode.as_str() {
        "fill" => fill(&args).await,
        "read" => read_cache(&args).await,
        "proxy" => read_proxy(&args).await,
        "touch" => touch_all(&args).await,
        other => eprintln!("usage: cache_bench fill|read|proxy --dir D [--gib G] [--budget-gib B] [--repeats N] [--samples N] [--slow-ms M] [--churn PER_SECOND] [--workers N] [--blocking N] (got {other:?})"),
    }
}
