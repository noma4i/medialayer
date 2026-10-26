use super::*;
use super::ingress::{IngressLimits, Pause, PlaceInput};
use crate::api::test_support::{assembly, eventually, eventually_async, shared, slow_assembly, FakeSettings, MockFactory, MockSlot};
use crate::api::{MediaError, MediaItem, MediaState, MediaStats, PlayerSlotFactory, SlotCall, LIMITS_KEY};
use crate::fetch::mock::MockFetcher;
use crate::proxy::test_client;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;
use url::Url;

struct Offline;

#[async_trait]
impl Fetcher for Offline {
    async fn fetch(&self, _request: super::FetchRequest) -> Result<super::FetchResponse, MediaError> {
        Err(MediaError::Fetch { detail: "offline".to_string() })
    }
}

struct Served(Mutex<Option<Vec<u8>>>);

#[async_trait]
impl BodyReader for Served {
    async fn next_chunk(&self) -> Result<Option<Vec<u8>>, MediaError> {
        Ok(self.0.lock().unwrap().take())
    }

    fn release(&self) {}
}

#[derive(Default)]
struct Origin {
    seen: Mutex<Vec<super::FetchRequest>>,
}

#[async_trait]
impl Fetcher for Origin {
    async fn fetch(&self, request: super::FetchRequest) -> Result<super::FetchResponse, MediaError> {
        self.seen.lock().unwrap().push(request);
        let body = Arc::new(Served(Mutex::new(Some(b"origin bytes".to_vec()))));
        Ok(super::FetchResponse { status: 200, headers: HashMap::from([("Content-Type".to_string(), "video/mp4".to_string())]), body })
    }
}

struct Seen(Mutex<Vec<MediaState>>);

impl StateListener for Seen {
    fn changed(&self, state: MediaState) {
        self.0.lock().unwrap().push(state);
    }

    fn failed(&self, _: MediaError) {}
}

struct Totals(Mutex<Vec<u64>>);

impl StatsListener for Totals {
    fn changed(&self, stats: MediaStats) {
        self.0.lock().unwrap().push(stats.total_bytes);
    }

    fn failed(&self, _: MediaError) {}
}

fn config(dir: &std::path::Path, fetcher: Arc<dyn Fetcher>, settings: Arc<FakeSettings>, slots: Option<Arc<dyn PlayerSlotFactory>>) -> OpenConfig {
    OpenConfig {
        available_bytes: u64::MAX,
        dir: dir.to_string_lossy().to_string(),
        fetcher,
        settings,
        slots,
        device: DeviceMemory { total_mb: 4096, heap_class_mb: None, low_ram: false },
        defaults: default_limits(),
        pool: default_pool_settings(),
        namespace_headers: Vec::new(),
        image_windows: Vec::new(),
        forward_headers: None,
        evict_video: None,
        evict_image: None,
        evict_audio: None,
        memory_releaser: None,
        silent: false,
        ingress: None,
    }
}

fn ring(layer: &MediaLayer, active: u32, sources: Vec<Option<MediaItem>>) -> Arc<MediaWindow> {
    let window = layer.open_video_window().unwrap();
    window.replace(active, sources).unwrap();
    window
}

fn place(layer: &MediaLayer, url: &str, input: PlaceInput) -> Arc<MediaPlace> {
    let place = layer.open_place(MediaItem::bare_video(url)).unwrap();
    place.update(input).unwrap();
    place
}

fn playing() -> PlaceInput {
    PlaceInput { playing: true, ..PlaceInput::default() }
}

fn caller() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

#[test]
fn inv_spec_07_open_applies_the_first_video_forward_headers_to_origin() {
    let dir = tempfile::tempdir().unwrap();
    let origin = Arc::new(Origin::default());
    let parsed = spec_defaults(r#"{"videos":[{"name":"first","forward_headers":["X-Token"]},{"name":"second","forward_headers":["Authorization"]}]}"#.to_string()).unwrap();
    let mut open = config(dir.path(), origin.clone(), shared(), None);
    open.defaults = parsed.defaults;
    open.pool = parsed.pool;
    open.forward_headers = parsed.forward_headers;
    let layer = MediaLayer::open(open).unwrap();
    caller().block_on(async {
        let bytes = layer.fetch_image("https://o.test/p.jpg".to_string(), HashMap::from([
            ("X-Token".to_string(), "selected".to_string()),
            ("Authorization".to_string(), "excluded".to_string()),
        ])).await.unwrap();
        assert_eq!(bytes.bytes, b"origin bytes");
        assert_eq!(origin.seen.lock().unwrap()[0].headers, HashMap::from([("X-Token".to_string(), "selected".to_string()), ("Accept-Encoding".to_string(), "identity".to_string())]));
    });
}

#[tokio::test]
async fn inv_spec_07_assembly_applies_the_first_eviction_override_of_each_kind() {
    for (section, kind) in [("videos", crate::cache::Kind::Stream), ("images", crate::cache::Kind::Image), ("audios", crate::cache::Kind::Audio)] {
        let dir = tempfile::tempdir().unwrap();
        let parsed = spec_defaults(format!(r#"{{"cache":{{"budget":10,"min":1,"evict":"lru"}},"{section}":[{{"name":"first","evict":"largest_first"}},{{"name":"second","evict":"fifo"}}]}}"#)).unwrap();
        let mut config = assembly(dir.path(), Arc::new(MockFetcher::new()), shared());
        config.defaults = parsed.defaults.try_into().unwrap();
        config.pool = parsed.pool.into();
        config.eviction_overrides = [
            (crate::cache::Kind::Stream, parsed.evict_video),
            (crate::cache::Kind::Image, parsed.evict_image),
            (crate::cache::Kind::Audio, parsed.evict_audio),
        ].into_iter().filter_map(|(kind, evict)| evict.map(|evict| (kind, evict.into()))).collect();
        let parts = crate::api::assemble(config).await.unwrap();
        let cache = &parts.cache;
        cache.ready().await;
        cache.put("small", kind, &[1; 2]).await.unwrap();
        cache.put("large", kind, &[2; 7]).await.unwrap();
        cache.put("new", kind, &[3; 4]).await.unwrap();
        assert!(cache.contains("small").await, "{section} evicted the oldest small entry");
        assert!(!cache.contains("large").await);
        assert!(cache.contains("new").await);
    }
}

#[tokio::test]
async fn inv_ffi_02_saved_limits_are_read_once_by_the_assembly_and_never_again() {
    let settings: Arc<FakeSettings> = shared();
    let dir = tempfile::tempdir().unwrap();
    let mut assembled = assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>());
    assembled.settings = settings.clone();
    let reads = || settings.reads.load(std::sync::atomic::Ordering::SeqCst);

    let parts = crate::api::assemble(assembled).await.unwrap();
    assert_eq!(reads(), 1);

    let layer = MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new());
    assert_eq!(reads(), 1);
    layer.limits().await.unwrap();
    layer.stats().await.unwrap();
    assert_eq!(reads(), 1);
}

#[test]
fn inv_ffi_02_open_wires_the_real_parts_so_limits_reach_the_store_and_stats_answer() {
    let dir = tempfile::tempdir().unwrap();
    let settings: Arc<FakeSettings> = shared();
    let caller = caller();

    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), settings.clone(), None)).unwrap();
    let smaller = Limits { budget: 512 * 1024 * 1024, ..default_limits() };
    let (limits, stats) = caller.block_on(async {
        layer.set_limits(smaller.clone()).await.unwrap();
        (layer.limits().await.unwrap(), layer.stats().await.unwrap())
    });

    assert_eq!(limits, smaller);
    assert!(settings.writes.lock().unwrap().iter().any(|(key, value)| key == LIMITS_KEY && value.contains("budget=536870912")));
    assert_eq!((stats.total_bytes, stats.file_count), (0, 0));
}

#[test]
fn inv_ffi_02_a_play_url_routes_the_platform_fetcher_through_the_proxy_into_the_disk_cache_and_purge_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let origin = Arc::new(Origin::default());
    let layer = MediaLayer::open(config(dir.path(), origin.clone(), shared::<FakeSettings>(), None)).unwrap();
    let source = MediaItem {
        headers: HashMap::from([("Authorization".to_string(), "token".to_string())]),
        ..MediaItem::bare_video("https://o.test/clip.mp4")
    };
    let caller = caller();

    let route = caller.block_on(layer.play_url(source.clone())).unwrap();
    let parsed = Url::parse(&route.url).unwrap();
    let port = parsed.port().unwrap();
    let reply = caller.block_on(test_client::get(port, &format!("{}?{}", parsed.path(), parsed.query().unwrap_or_default())));
    let stored = caller.block_on(async {
        for _ in 0..1_000_000 {
            if layer.stats().await.unwrap().file_count > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        layer.stats().await.unwrap().file_count
    });
    caller.block_on(layer.purge(source.url.clone())).unwrap();

    assert_eq!(reply.body, b"origin bytes");
    assert_eq!(origin.seen.lock().unwrap()[0].headers.get("Authorization").map(String::as_str), Some("token"));
    assert_eq!(stored, 1);
    assert_eq!(caller.block_on(layer.stats()).unwrap().file_count, 0);
    assert_eq!(caller.block_on(layer.refresh_route(source, route.clone())).unwrap(), route);
}

#[test]
fn inv_ffi_02_declarations_reach_the_platform_slots_and_the_player_events_move_the_state() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), Some(slots))).unwrap();
    let url = "https://o.test/v1.mp4";
    let seen = Arc::new(Seen(Mutex::new(Vec::new())));
    let _subscription = layer.subscribe_state(url.to_string(), seen.clone()).unwrap();
    let caller = caller();

    let _ring = ring(&layer, 1, (0..3).map(|index| Some(MediaItem::bare_video(&format!("https://o.test/v{index}.mp4")))).collect());
    layer.set_muted_all(true).unwrap();
    let _place = place(&layer, url, PlaceInput { playing: true, visible: true, ..PlaceInput::default() });
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))));
    let (index, slot): (u32, Arc<MockSlot>) = factory.slot_holding(url).unwrap();
    layer.player_events().ready(index, slot.generation());

    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Ready)));
    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Mute)));
    layer.release_all().unwrap();
    caller.block_on(eventually(|| slot.calls().last() == Some(&SlotCall::Unload)));
}

#[test]
fn inv_ffi_02_a_cache_write_reaches_a_stats_subscriber_without_any_controls_call() {
    let dir = tempfile::tempdir().unwrap();
    let caller = caller();
    let (layer, cache) = caller.block_on(async {
        let assembled = assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>());
        let parts = crate::api::assemble(assembled).await.unwrap();
        let cache = parts.cache.clone();
        (MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new()), cache)
    });
    let totals = Arc::new(Totals(Mutex::new(Vec::new())));
    let _subscription = layer.subscribe_stats(totals.clone()).unwrap();

    caller.block_on(async {
        cache.put("https://o.test/a.jpg", crate::cache::Kind::Image, &[1; 500]).await.unwrap();
        eventually_async(|| async { totals.0.lock().unwrap().last() == Some(&500) }).await;
    });
}

#[derive(Default)]
struct CountingReleaser(std::sync::atomic::AtomicUsize);

#[async_trait]
impl MemoryReleaser for CountingReleaser {
    async fn shutdown(&self) {}

    fn drop_memory_caches(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn inv_ffi_02_a_critical_memory_warning_reaches_the_platform_memory_releaser() {
    let dir = tempfile::tempdir().unwrap();
    let releaser: Arc<CountingReleaser> = shared();
    let mut open = config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), None);
    open.memory_releaser = Some(releaser.clone());
    let layer = MediaLayer::open(open).unwrap();

    layer.player_events().memory_warning(crate::api::Pressure::Critical);

    for _ in 0..2_000_000 {
        if releaser.0.load(std::sync::atomic::Ordering::SeqCst) == 1 {
            break;
        }
        std::thread::yield_now();
    }
    assert_eq!(releaser.0.load(std::sync::atomic::Ordering::SeqCst), 1);
}

fn origin_layer(dir: &std::path::Path) -> (Arc<MediaLayer>, Arc<Origin>) {
    let origin = Arc::new(Origin::default());
    (MediaLayer::open(config(dir, origin.clone(), shared::<FakeSettings>(), None)).unwrap(), origin)
}

#[test]
fn inv_ffi_03_an_image_loads_through_the_fetcher_into_the_cache_once_and_repeats_come_from_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let (layer, origin) = origin_layer(dir.path());
    let headers = HashMap::from([("Authorization".to_string(), "token".to_string())]);
    let caller = caller();

    let (first, second, together) = caller.block_on(async {
        let first = layer.fetch_image("https://o.test/a.jpg".to_string(), headers.clone()).await;
        let second = layer.fetch_image("https://o.test/a.jpg".to_string(), headers.clone()).await;
        let together = tokio::join!(
            layer.fetch_image("https://o.test/b.jpg".to_string(), HashMap::new()),
            layer.fetch_image("https://o.test/b.jpg".to_string(), HashMap::new())
        );
        (first, second, together)
    });

    assert_eq!((first.unwrap().bytes, second.unwrap().bytes), (b"origin bytes".to_vec(), b"origin bytes".to_vec()));
    assert_eq!((together.0.unwrap().bytes, together.1.unwrap().bytes), (b"origin bytes".to_vec(), b"origin bytes".to_vec()));
    let seen = origin.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].headers.get("Authorization").map(String::as_str), Some("token"));
    assert!(seen.iter().all(|request| request.range_start.is_none()));
}

#[test]
fn inv_ffi_03_an_image_that_cannot_be_fetched_is_a_fetch_error() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), None)).unwrap();

    let result = caller().block_on(layer.fetch_image("https://o.test/a.jpg".to_string(), HashMap::new()));

    assert!(matches!(result, Err(MediaError::Fetch { .. })));
}

#[test]
fn inv_placeholder_09_a_cached_video_keeps_its_cover_until_the_first_frame_is_shown() {
    let dir = tempfile::tempdir().unwrap();
    let caller = caller();
    let (layer, cache) = caller.block_on(async {
        let parts = crate::api::assemble(assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>())).await.unwrap();
        let cache = parts.cache.clone();
        (MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new()), cache)
    });
    let cached = MediaItem { cover: Some("https://o.test/cover.jpg".to_string()), ..MediaItem::bare_video("https://o.test/cached.mp4") };
    let cover = crate::api::Placeholder::Cover { url: "https://o.test/cover.jpg".to_string() };

    caller.block_on(cache.put("https://o.test/cached.mp4#0", crate::cache::Kind::Stream, b"first block")).unwrap();

    assert!(caller.block_on(layer.is_cached(cached.clone())));
    assert_eq!(caller.block_on(layer.placeholder(cached.clone(), false)), cover);
    assert_eq!(caller.block_on(layer.placeholder(cached, true)), crate::api::Placeholder::None);
}

#[test]
fn inv_placeholder_08_no_placeholder_reads_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let caller = caller();
    let (layer, routes) = caller.block_on(async {
        let parts = crate::api::assemble(assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>())).await.unwrap();
        let routes = parts.routes.clone();
        (MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new()), routes)
    });
    let item = |url: &str, kind: crate::api::Kind| MediaItem::bare(url, kind);
    let cases = [
        ("image", item("https://o.test/a.jpg", crate::api::Kind::Image), false),
        ("ready image", item("https://o.test/b.jpg", crate::api::Kind::Image), true),
        ("audio", item("https://o.test/a.mp3", crate::api::Kind::Audio), false),
        ("ready video", item("https://o.test/a.mp4", crate::api::Kind::Video), true),
        ("unready video", item("https://o.test/b.mp4", crate::api::Kind::Video), false),
    ];
    for (name, source, ready) in cases {
        let before = routes.storage_lookups();
        caller.block_on(layer.placeholder(source, ready));
        assert_eq!(routes.storage_lookups() - before, 0, "{name}");
    }
}

#[test]
fn inv_ffi_03_an_item_state_from_a_native_loader_reaches_the_state_subscribers_and_a_seek_reaches_the_player() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), Some(slots))).unwrap();
    let seen = Arc::new(Seen(Mutex::new(Vec::new())));
    let _subscription = layer.subscribe_state("https://o.test/a.jpg".to_string(), seen.clone()).unwrap();
    let caller = caller();
    let image = layer.open_place(MediaItem::bare("https://o.test/a.jpg", crate::api::Kind::Image)).unwrap();

    image.item_state(MediaState::Loading).unwrap();
    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Loading)));
    image.item_state(MediaState::Ready).unwrap();
    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Ready)));

    let url = "https://o.test/v1.mp4";
    let _ring = ring(&layer, 0, vec![Some(MediaItem::bare_video(url))]);
    let video = place(&layer, url, PlaceInput { view_token: Some(5), ..PlaceInput::default() });
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    video.seek(0.025).unwrap();
    let (_, player) = factory.slot_holding(url).unwrap();
    caller.block_on(eventually(|| player.calls().contains(&SlotCall::Seek { position_ms: 1_500 }) && player.calls().contains(&SlotCall::Attach { view_token: 5 })));
}

#[test]
fn inv_ffi_05_open_takes_the_spec_defaults_without_writing_them_and_a_saved_limit_wins_over_them() {
    let spec = crate::spec::MediaSpec {
        cache: crate::spec::CacheSpec { budget: 3_000_000_000, ..Default::default() },
        videos: vec![crate::spec::VideoSpec { pool: crate::spec::PoolSpec { slots: 5, ..Default::default() }, ..crate::spec::VideoSpec::new("clip") }],
        ..Default::default()
    };
    let parsed = crate::ffi::spec_defaults(spec.to_json()).unwrap();
    let settings: Arc<FakeSettings> = shared();
    let caller = caller();
    let from_spec = |dir: &std::path::Path| {
        let mut open = config(dir, Arc::new(Offline), settings.clone(), None);
        open.defaults = parsed.defaults.clone();
        open.pool = parsed.pool;
        MediaLayer::open(open).unwrap()
    };
    let first_dir = tempfile::tempdir().unwrap();

    let layer = from_spec(first_dir.path());
    let limits = caller.block_on(layer.limits()).unwrap();

    assert_eq!((limits.budget, limits.slots), (3_000_000_000, 5));
    assert!(settings.writes.lock().unwrap().is_empty());
    let saved = Limits { budget: 600_000_000, ..parsed.defaults.clone() };
    caller.block_on(layer.set_limits(saved.clone())).unwrap();
    drop(layer);
    let second_dir = tempfile::tempdir().unwrap();
    assert_eq!(caller.block_on(from_spec(second_dir.path()).limits()).unwrap(), saved);
}

#[test]
fn inv_ffi_05_namespace_headers_given_to_open_give_each_header_value_its_own_cache_entry() {
    let caller = caller();
    let fetch_twice = |namespace: Vec<String>| {
        let dir = tempfile::tempdir().unwrap();
        let origin = Arc::new(Origin::default());
        let mut open = config(dir.path(), origin.clone(), shared::<FakeSettings>(), None);
        open.namespace_headers = namespace;
        let layer = MediaLayer::open(open).unwrap();
        for token in ["one", "two"] {
            let headers = HashMap::from([("Authorization".to_string(), token.to_string())]);
            caller.block_on(layer.fetch_image("https://o.test/a.jpg".to_string(), headers)).unwrap();
        }
        let seen = origin.seen.lock().unwrap().len();
        seen
    };

    assert_eq!((fetch_twice(Vec::new()), fetch_twice(vec!["Authorization".to_string()])), (1, 2));
}

#[test]
fn inv_api_15_the_placeholder_of_a_local_file_is_empty_even_when_it_names_a_cover() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), None)).unwrap();
    let local = MediaItem { cover: Some("https://o.test/cover.jpg".to_string()), ..MediaItem::bare_video("file:///bundle/intro.mp4") };
    let remote = MediaItem { cover: Some("https://o.test/cover.jpg".to_string()), ..MediaItem::bare_video("https://o.test/a.mp4") };

    assert_eq!(caller().block_on(layer.placeholder(local, false)), crate::api::Placeholder::None);
    assert_eq!(caller().block_on(layer.placeholder(remote, false)), crate::api::Placeholder::Cover { url: "https://o.test/cover.jpg".to_string() });
}

fn poster_pages(count: usize) -> Vec<Vec<MediaItem>> {
    (0..count).map(|index| vec![MediaItem::bare(&format!("https://o.test/p{index}.jpg"), crate::api::Kind::Image)]).collect()
}

#[test]
fn inv_ffi_06_an_image_prefetched_by_a_window_is_a_cache_hit_for_fetch_image() {
    let dir = tempfile::tempdir().unwrap();
    let (layer, origin) = origin_layer(dir.path());
    let caller = caller();

    let grid = layer.open_image_window("grid".to_string(), 1, false).unwrap();
    grid.replace_pages(0, poster_pages(1)).unwrap();
    caller.block_on(eventually(|| !origin.seen.lock().unwrap().is_empty()));
    let fetched = caller.block_on(layer.fetch_image("https://o.test/p0.jpg".to_string(), HashMap::new()));

    assert_eq!(fetched.unwrap().bytes, b"origin bytes".to_vec());
    assert_eq!(origin.seen.lock().unwrap().len(), 1);
}

const SLOW: std::time::Duration = std::time::Duration::from_secs(2);
const CALLER_BUDGET: std::time::Duration = std::time::Duration::from_millis(100);

#[test]
fn inv_ffi_07_open_returns_to_the_caller_while_the_assembly_is_still_running() {
    let dir = tempfile::tempdir().unwrap();
    slow_assembly(dir.path(), SLOW);
    let started = std::time::Instant::now();

    let _layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), None)).unwrap();

    assert!(started.elapsed() < CALLER_BUDGET, "open held the caller for {:?}", started.elapsed());
}

#[test]
fn inv_ffi_07_calls_made_before_the_assembly_ends_run_after_it_in_call_order_and_are_not_lost() {
    let dir = tempfile::tempdir().unwrap();
    slow_assembly(dir.path(), SLOW);
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let started = std::time::Instant::now();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), Some(slots))).unwrap();
    let url = "https://o.test/v1.mp4";
    let seen = Arc::new(Seen(Mutex::new(Vec::new())));
    let _subscription = layer.subscribe_state(url.to_string(), seen.clone()).unwrap();
    let caller = caller();

    let _ring = ring(&layer, 0, vec![Some(MediaItem::bare_video(url))]);
    let _place = place(&layer, url, PlaceInput { view_token: Some(5), ..playing() });
    let declared_in = started.elapsed();
    let limits = caller.block_on(layer.limits()).unwrap();

    assert!(declared_in < SLOW / 2, "the declarations waited {declared_in:?} for the assembly");
    assert!(started.elapsed() >= SLOW, "the async call returned before the assembly ended");
    assert_eq!(limits, default_limits());
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play) && slot.calls().contains(&SlotCall::Attach { view_token: 5 }))));
    let (index, slot) = factory.slot_holding(url).unwrap();
    layer.player_events().ready(index, slot.generation());
    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Ready)));
}

const IDLE: std::time::Duration = std::time::Duration::from_secs(10);
const IDLE_STEP: std::time::Duration = std::time::Duration::from_millis(50);

async fn elapse(span: std::time::Duration) {
    for _ in 0..span.as_millis() / IDLE_STEP.as_millis() {
        tokio::time::advance(IDLE_STEP).await;
    }
}

async fn idle_wakes(declare: bool) -> (u64, u64, usize) {
    let dir = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(MockFetcher::default());
    let factory = shared::<MockFactory>();
    let config = assembly(dir.path(), fetcher.clone(), factory.clone());
    let parts = crate::api::assemble(config).await.unwrap();
    parts.cache.ready().await;
    let mut settled = parts.prefetcher.settled();
    let assembled = super::assembled::Assembled::new(&Handle::current(), parts, None);
    if declare {
        let url = crate::api::test_support::url_of(0);
        assembled.stage.replace_window(0, crate::api::test_support::videos(3));
        assembled.stage.set_playing(&url, true, &[]);
        eventually(|| factory.slot_holding(&url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))).await;
        let mut pending: std::collections::HashSet<_> = (1..3).map(crate::api::test_support::url_of).collect();
        while !pending.is_empty() {
            pending.remove(&settled.recv().await.unwrap().url);
        }
    }
    elapse(std::time::Duration::from_secs(1)).await;
    let (stats, tick, requests) = (assembled.controls.wakes(), assembled.stage.wakes(), fetcher.calls());
    elapse(IDLE).await;
    (assembled.controls.wakes() - stats, assembled.stage.wakes() - tick, fetcher.calls() - requests)
}

#[tokio::test(start_paused = true)]
async fn inv_idle_01_idle_census_shows_no_timer_task_or_request_in_ten_seconds() {
    assert_eq!(idle_wakes(false).await, (0, 0, 0), "nothing declared");
    assert_eq!(idle_wakes(true).await, (0, 0, 0), "a window declared and a video playing");
}

#[tokio::test(start_paused = true)]
async fn inv_idle_02_stats_are_published_by_change_events_and_only_with_a_subscriber() {
    let dir = tempfile::tempdir().unwrap();
    let parts = crate::api::assemble(assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>())).await.unwrap();
    parts.cache.ready().await;
    let cache = parts.cache.clone();
    let assembled = super::assembled::Assembled::new(&Handle::current(), parts, None);
    for index in 0..200 {
        cache.put(&format!("https://o.test/{index}.jpg"), crate::cache::Kind::Image, &[1; 10]).await.unwrap();
    }
    elapse(std::time::Duration::from_secs(1)).await;
    assert_eq!(assembled.controls.wakes(), 0, "200 cache writes with no subscriber");

    let totals = Arc::new(Totals(Mutex::new(Vec::new())));
    let sink = totals.clone();
    let _subscription = assembled.controls.subscribe_stats(Arc::new(move |stats: MediaStats| sink.changed(stats)));
    elapse(std::time::Duration::from_secs(1)).await;
    eventually(|| totals.0.lock().unwrap().last() == Some(&2000)).await;
    let subscribed = assembled.controls.wakes();
    assert_eq!(subscribed, 1, "the subscription delivers the current stats once");

    cache.put("https://o.test/one.jpg", crate::cache::Kind::Image, &[1; 10]).await.unwrap();
    elapse(std::time::Duration::from_secs(1)).await;
    eventually(|| totals.0.lock().unwrap().last() == Some(&2010)).await;
    assert_eq!(assembled.controls.wakes() - subscribed, 1, "one write, one publication");

    for index in 0..100 {
        cache.put(&format!("https://o.test/burst{index}.jpg"), crate::cache::Kind::Image, &[1; 10]).await.unwrap();
    }
    elapse(std::time::Duration::from_secs(1)).await;
    // Virtual time does not complete callbacks on blocking threads.
    eventually(|| totals.0.lock().unwrap().last() == Some(&3010)).await;
    assert!(assembled.controls.wakes() - subscribed <= 3, "a burst of 100 writes merges: {}", assembled.controls.wakes() - subscribed);
    assert_eq!(totals.0.lock().unwrap().last(), Some(&(301 * 10)));
}

fn failed_layer(runtime: tokio::runtime::Runtime) -> Arc<MediaLayer> {
    MediaLayer::assembling(runtime, async { Err(MediaError::Unexpected { detail: "loopback bind failed".to_string() }) }, None, Vec::new(), IngressLimits::default(), Pause::new())
}

#[test]
fn inv_ffi_04_a_layer_that_failed_to_assemble_reports_the_error_to_the_calls_with_a_result() {
    let layer = failed_layer(tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap());
    let item = crate::api::test_support::video("https://o.test/a.mp4");

    caller().block_on(async {
        assert!(matches!(layer.stats().await, Err(MediaError::Unexpected { .. })));
        assert!(matches!(layer.limits().await, Err(MediaError::Unexpected { .. })));
        assert!(!layer.is_cached(item.clone()).await);
        assert!(matches!(layer.clear(None).await, Err(MediaError::Unexpected { .. })));
        assert!(matches!(layer.purge(item.url.clone()).await, Err(MediaError::Unexpected { .. })));
        assert!(matches!(layer.evict_now().await, Err(MediaError::Unexpected { .. })));
        assert!(matches!(layer.play_url(item).await, Err(MediaError::Unexpected { .. })));
    });
}

#[test]
fn inv_ffi_04_a_panicking_media_task_is_an_error_not_a_panic_across_the_bridge() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared(), None)).unwrap();

    let outcome = caller().block_on(layer.controlled(|_| async { panic!("a media task failed") }));

    assert!(matches!(outcome, Err(MediaError::Unexpected { .. })));
}

#[tokio::test]
async fn inv_ffi_09_cancelling_outer_call_drops_inner_work() {
    let dir = tempfile::tempdir().unwrap();
    let parts = crate::api::assemble(assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>())).await.unwrap();
    let layer = MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new());
    let (started, running) = tokio::sync::oneshot::channel();
    let (resource, released) = tokio::sync::oneshot::channel::<()>();
    let outer = tokio::spawn(async move {
        layer.controlled(|_| async move {
            let _resource = resource;
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        }).await
    });
    running.await.unwrap();
    outer.abort();
    assert!(outer.await.unwrap_err().is_cancelled());
    let release = tokio::time::timeout(std::time::Duration::from_secs(1), released).await;
    assert!(matches!(release, Ok(Err(_))), "inner work still owns its resource");
}

#[test]
fn inv_ffi_11_updates_before_the_assembly_fold_per_owner_and_stay_bounded() {
    let dir = tempfile::tempdir().unwrap();
    slow_assembly(dir.path(), SLOW);
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), Some(slots))).unwrap();
    let items: Vec<Option<MediaItem>> = (0..3).map(|index| Some(MediaItem::bare_video(&format!("https://o.test/v{index}.mp4")))).collect();
    let url = "https://o.test/v1.mp4";
    let window = layer.open_video_window().unwrap();
    let place = layer.open_place(MediaItem::bare_video(url)).unwrap();

    for round in 0..2_000u32 {
        window.replace(round % 3, items.clone()).unwrap();
        window.move_to(round % 3).unwrap();
        place.update(PlaceInput { playing: round % 2 == 0, muted: Some(round % 2 == 1), visible: true, ..PlaceInput::default() }).unwrap();
        place.item_state(MediaState::Loading).unwrap();
    }

    let audit = layer.queue().audit();
    assert!(audit.counted <= 8 && audit.owners == 2, "{audit:?} wait for an assembly that has not ended");
    let caller = caller();
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Mute))));
    let (_, slot) = factory.slot_holding(url).unwrap();
    assert!(!slot.calls().contains(&SlotCall::Play), "{:?}", slot.calls());
}

#[derive(Default)]
struct ClosingReleaser {
    shutdowns: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl MemoryReleaser for ClosingReleaser {
    fn drop_memory_caches(&self) {}

    async fn shutdown(&self) {
        self.shutdowns.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn inv_ffi_12_shutdown_releases_the_platform_then_the_core_once_whoever_holds_the_object() {
    let dir = tempfile::tempdir().unwrap();
    let releaser: Arc<ClosingReleaser> = shared();
    let mut open = config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), None);
    open.memory_releaser = Some(releaser.clone());
    let layer = MediaLayer::open(open).unwrap();
    let held = layer.clone();
    let caller = caller();
    caller.block_on(layer.stats()).unwrap();

    layer.shutdown();
    layer.shutdown();
    caller.block_on(layer.await_closed());

    assert_eq!(releaser.shutdowns.load(std::sync::atomic::Ordering::SeqCst), 1);
    let after = caller.block_on(async { tokio::time::timeout(std::time::Duration::from_secs(5), held.stats()).await });
    assert!(matches!(after, Ok(Err(MediaError::Closed))), "{after:?}");
    drop((layer, held));
    assert_eq!(releaser.shutdowns.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn inv_android_07_events_do_not_keep_a_closed_owner_alive() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), None)).unwrap();
    let caller = caller();
    caller.block_on(layer.limits()).unwrap();
    let queue = Arc::downgrade(layer.queue());
    let events = layer.player_events();

    layer.shutdown();
    caller.block_on(layer.await_closed());
    drop(layer);

    caller.block_on(crate::api::test_support::eventually(|| queue.upgrade().is_none()));
    events.ready(0, 0);
    events.foreground();
    assert!(queue.upgrade().is_none(), "a late event revived the closed owner");
}

#[test]
fn inv_ffi_17_closing_during_a_held_assembly_leaves_neither_the_queue_nor_the_owner_alive_after_the_build_exits() {
    use std::sync::mpsc;
    use std::time::Duration;
    let (entered_tx, entered) = mpsc::channel::<()>();
    let (release, released) = mpsc::channel::<()>();
    let (exited_tx, exited) = mpsc::channel::<()>();
    let build = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let held = async move {
        let _ = tokio::task::spawn_blocking(move || {
            entered_tx.send(()).ok();
            let _ = released.recv();
            exited_tx.send(()).ok();
        })
        .await;
        Err::<crate::api::Parts, MediaError>(MediaError::Unexpected { detail: "held assembly ended".to_string() })
    };
    let layer = MediaLayer::assembling(build, held, None, Vec::new(), IngressLimits::default(), Pause::new());
    entered.recv_timeout(Duration::from_secs(10)).expect("the assembly never started");
    let queue = Arc::downgrade(layer.queue());
    let owner = layer.closing_ref();
    let events = layer.player_events();
    let caller = caller();

    layer.shutdown();
    caller.block_on(layer.await_closed());
    drop(layer);
    events.ready(0, 0);
    events.foreground();
    release.send(()).unwrap();
    exited.recv_timeout(Duration::from_secs(10)).expect("the held assembly never exited");

    caller.block_on(eventually(|| queue.upgrade().is_none() && owner.upgrade().is_none()));
    events.ready(0, 0);
    events.background();
    assert!(queue.upgrade().is_none() && owner.upgrade().is_none(), "a late event revived the closed owner");
}

#[test]
fn inv_ffi_12_shutdown_releases_the_assembled_value_whoever_still_holds_the_layer() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), None)).unwrap();
    let caller = caller();
    let assembled = caller.block_on(layer.controlled(|assembled| async move { Arc::downgrade(&assembled) })).unwrap();
    let suspension = layer.suspend_playback(Vec::new()).unwrap();
    let events = layer.player_events();
    assert!(assembled.strong_count() >= 1, "the assembled value is alive while the layer is open");

    layer.shutdown();
    caller.block_on(layer.await_closed());

    assert_eq!(assembled.strong_count(), 0, "the closed layer, its suspension or its events still own the assembled value");
    suspension.release();
    events.ready(0, 0);
    assert!(layer.set_muted_all(true).is_err());
    assert!(layer.open_place(MediaItem::bare_video("https://o.test/late.mp4")).is_err());
    assert_eq!(assembled.strong_count(), 0, "a call after the close revived the assembled value");
    drop((layer, suspension, events));
}

#[test]
fn inv_ffi_12_shutdown_answers_a_call_that_waits_for_the_assembly_with_closed() {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    let (started, begun) = tokio::sync::oneshot::channel::<()>();
    let layer = MediaLayer::assembling(
        runtime,
        async move {
            let _ = started.send(());
            std::future::pending::<Result<crate::api::Parts, MediaError>>().await
        },
        None,
        Vec::new(),
        IngressLimits::default(),
        Pause::new(),
    );

    let outcome = caller().block_on(async {
        begun.await.unwrap();
        layer.shutdown();
        tokio::time::timeout(std::time::Duration::from_secs(2), layer.stats()).await
    });

    match outcome {
        Err(_) => panic!("stats() never answered after the shutdown of a layer still assembling"),
        Ok(result) => assert!(matches!(result, Err(MediaError::Closed)), "stats() answered {result:?}"),
    }
}

#[test]
fn inv_ffi_13_a_whole_source_shown_before_the_assembly_downloads_after_it() {
    let dir = tempfile::tempdir().unwrap();
    slow_assembly(dir.path(), SLOW);
    let origin = Arc::new(Origin::default());
    let layer = MediaLayer::open(config(dir.path(), origin.clone(), shared::<FakeSettings>(), None)).unwrap();
    let clip = MediaItem { strategy: crate::prefetch::CacheStrategy::Whole, ..MediaItem::bare_video("https://o.test/clip.mp4") };

    let shown = layer.open_place(clip).unwrap();
    shown.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();

    caller().block_on(eventually(|| !origin.seen.lock().unwrap().is_empty()));
}

#[test]
fn inv_ffi_13_a_whole_source_of_each_window_before_the_assembly_downloads_after_it() {
    let dir = tempfile::tempdir().unwrap();
    slow_assembly(dir.path(), SLOW);
    let origin = Arc::new(Origin::default());
    let layer = MediaLayer::open(config(dir.path(), origin.clone(), shared::<FakeSettings>(), None)).unwrap();
    let clip = MediaItem { strategy: crate::prefetch::CacheStrategy::Whole, ..MediaItem::bare_video("https://o.test/clip.mp4") };
    let photo = MediaItem { strategy: crate::prefetch::CacheStrategy::Whole, ..MediaItem::bare("https://o.test/photo.jpg", crate::api::Kind::Image) };

    let _ring = ring(&layer, 0, vec![Some(clip)]);
    let feed = layer.open_image_window("feed".to_string(), 1, false).unwrap();
    feed.replace_pages(0, vec![vec![photo]]).unwrap();

    caller().block_on(eventually(|| {
        let seen = origin.seen.lock().unwrap();
        ["clip.mp4", "photo.jpg"].iter().all(|name| seen.iter().any(|request| request.url.contains(name)))
    }));
}

fn transport(slot: &MockSlot) -> Option<SlotCall> {
    slot.calls().into_iter().rfind(|call| matches!(call, SlotCall::Play | SlotCall::Pause))
}

#[test]
fn inv_session_13_a_blank_allow_group_is_rejected_and_suspends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), None)).unwrap();
    assert!(matches!(layer.suspend_playback(vec!["chat".to_string(), "  ".to_string()]), Err(MediaError::InvalidSuspension { .. })));
    assert!(layer.suspend_playback(vec!["chat".to_string()]).is_ok());
}

#[test]
fn inv_session_12_release_acts_once_and_dropping_the_handle_releases() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), Some(slots))).unwrap();
    let url = "https://o.test/v1.mp4";
    let caller = caller();
    let _ring = ring(&layer, 0, vec![Some(MediaItem::bare_video(url))]);
    let place = place(&layer, url, playing());
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| transport(&slot) == Some(SlotCall::Play))));
    let (_, slot) = factory.slot_holding(url).unwrap();

    let first = layer.suspend_playback(Vec::new()).unwrap();
    let second = layer.suspend_playback(Vec::new()).unwrap();
    caller.block_on(eventually(|| transport(&slot) == Some(SlotCall::Pause)));

    for _ in 0..3 {
        first.release();
    }
    place.update(PlaceInput { rate: Some(2.0), ..playing() }).unwrap();
    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Rate { rate: 2.0 })));
    assert_eq!(transport(&slot), Some(SlotCall::Pause), "three releases of one handle released the other");

    drop(second);
    caller.block_on(eventually(|| transport(&slot) == Some(SlotCall::Play)));
}

#[test]
fn inv_session_13_a_suspension_taken_and_released_before_the_assembly_ends_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    slow_assembly(dir.path(), SLOW);
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), Some(slots))).unwrap();
    let url = "https://o.test/v1.mp4";
    let caller = caller();
    let _ring = ring(&layer, 0, vec![Some(MediaItem::bare_video(url))]);
    let _place = place(&layer, url, playing());
    layer.suspend_playback(Vec::new()).unwrap().release();
    let held = layer.suspend_playback(Vec::new()).unwrap();
    held.release();
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| transport(&slot) == Some(SlotCall::Play))));
}

fn opened(dir: &std::path::Path, factory: &Arc<MockFactory>, releaser: Option<Arc<dyn MemoryReleaser>>) -> Arc<MediaLayer> {
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let mut open = config(dir, Arc::new(Offline), shared::<FakeSettings>(), Some(slots));
    open.memory_releaser = releaser;
    MediaLayer::open(open).unwrap()
}

async fn pending_once(closed: impl std::future::Future<Output = ()>) -> bool {
    tokio::select! {
        biased;
        () = closed => false,
        () = std::future::ready(()) => true,
    }
}

fn disposed_all(factory: &MockFactory) -> bool {
    factory.disposed_slots() == factory.created.lock().unwrap().len()
}

#[test]
fn inv_ffi_16_closing_refuses_input_at_once_and_closed_comes_only_after_every_native_dispose() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let releaser: Arc<ClosingReleaser> = shared();
    let layer = opened(dir.path(), &factory, Some(releaser.clone()));
    let url = "https://o.test/v1.mp4";
    let caller = caller();
    let ring = ring(&layer, 0, vec![Some(MediaItem::bare_video(url))]);
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let (_, slot) = factory.slot_holding(url).unwrap();
    let events = layer.player_events();
    let gate = factory.dispose_gate.hold();

    layer.shutdown();
    let created = factory.created.lock().unwrap().len();
    assert!(ring.replace(0, (0..3).map(|index| Some(MediaItem::bare_video(&format!("https://o.test/late{index}.mp4")))).collect()).is_err());
    assert!(layer.open_place(MediaItem::bare_video("https://o.test/late.mp4")).is_err());
    events.ready(0, slot.generation());

    caller.block_on(eventually(|| factory.dispose_gate.entered() == 1));
    assert!(caller.block_on(pending_once(layer.await_closed())), "closed came before the native dispose ended");
    assert_eq!(releaser.shutdowns.load(std::sync::atomic::Ordering::SeqCst), 0, "the platform resources were released before the slots");
    drop(gate);
    caller.block_on(layer.await_closed());

    assert!(disposed_all(&factory) && slot.disposed.load(std::sync::atomic::Ordering::SeqCst) == 1);
    assert_eq!(factory.created.lock().unwrap().len(), created, "input after the shutdown made a player");
    assert_eq!(releaser.shutdowns.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(!layer.owns_runtime(), "the closed layer still owns its runtime");
}

#[test]
fn inv_ffi_16_a_held_native_port_blocks_neither_the_shutdown_nor_the_drop_and_a_cancelled_wait_keeps_the_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let layer = opened(dir.path(), &factory, None);
    let url = "https://o.test/v1.mp4";
    let caller = caller();
    let _ring = ring(&layer, 0, vec![Some(MediaItem::bare_video(url))]);
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let gate = factory.dispose_gate.hold();

    let started = std::time::Instant::now();
    assert!(caller.block_on(pending_once(layer.await_closed())), "await_closed does not start the close itself");
    drop(layer);
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "the caller waited for the held native port");

    caller.block_on(eventually(|| factory.dispose_gate.entered() == 1));
    drop(gate);
    caller.block_on(eventually(|| disposed_all(&factory)));
}

#[test]
fn inv_ffi_16_dropping_the_layer_starts_the_same_close_without_any_await() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let releaser: Arc<ClosingReleaser> = shared();
    let layer = opened(dir.path(), &factory, Some(releaser.clone()));
    let url = "https://o.test/v1.mp4";
    let caller = caller();
    let ring = ring(&layer, 0, vec![Some(MediaItem::bare_video(url))]);
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));

    drop((ring, layer));

    caller.block_on(eventually(|| disposed_all(&factory) && releaser.shutdowns.load(std::sync::atomic::Ordering::SeqCst) == 1));
}

struct Dropped(Arc<std::sync::atomic::AtomicBool>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn inv_ffi_17_closed_means_the_cancelled_assembly_future_is_already_dropped() {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (started, begun) = tokio::sync::oneshot::channel::<()>();
    let held = Dropped(dropped.clone());
    let layer = MediaLayer::assembling(
        runtime,
        async move {
            let _held = held;
            let _ = started.send(());
            std::future::pending::<Result<crate::api::Parts, MediaError>>().await
        },
        None,
        Vec::new(),
        IngressLimits::default(),
        Pause::new(),
    );

    caller().block_on(async {
        begun.await.unwrap();
        layer.await_closed().await;
    });

    assert!(dropped.load(std::sync::atomic::Ordering::SeqCst), "Closed came before the cancelled assembly released what it held");
}

#[test]
fn inv_ffi_17_closing_before_the_assembly_ends_cancels_it_and_still_releases_the_platform() {
    let dir = tempfile::tempdir().unwrap();
    slow_assembly(dir.path(), std::time::Duration::from_secs(60));
    let factory: Arc<MockFactory> = Arc::default();
    let releaser: Arc<ClosingReleaser> = shared();
    let layer = opened(dir.path(), &factory, Some(releaser.clone()));
    let _ring = ring(&layer, 0, vec![Some(MediaItem::bare_video("https://o.test/v1.mp4"))]);

    layer.shutdown();

    let closed = caller().block_on(async { tokio::time::timeout(std::time::Duration::from_secs(10), layer.await_closed()).await });
    assert!(closed.is_ok(), "the assembly kept the closing waiting");
    assert!(factory.created.lock().unwrap().is_empty());
    assert_eq!(releaser.shutdowns.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(matches!(caller().block_on(layer.stats()), Err(MediaError::Closed)));
}

/// Time for the core to act on an event that was already delivered.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Default)]
struct ReadySeen(Mutex<Vec<bool>>);

impl WindowReadyListener for ReadySeen {
    fn changed(&self, ready: bool) {
        self.0.lock().unwrap().push(ready);
    }
}

impl ReadySeen {
    fn is_ready(&self) -> bool {
        self.0.lock().unwrap().last() == Some(&true)
    }
}

/// Serves every url at once except the ones named `slow`, which wait until the test opens them.
#[derive(Default)]
struct Gated {
    open: Mutex<std::collections::HashSet<String>>,
    changed: tokio::sync::Notify,
}

impl Gated {
    fn release(&self, name: &str) {
        self.open.lock().unwrap().insert(format!("https://o.test/{name}.jpg"));
        self.changed.notify_waiters();
    }
}

#[async_trait]
impl Fetcher for Gated {
    async fn fetch(&self, request: super::FetchRequest) -> Result<super::FetchResponse, MediaError> {
        while request.url.contains("slow") {
            let changed = self.changed.notified();
            if self.open.lock().unwrap().contains(&request.url) {
                break;
            }
            changed.await;
        }
        let body = Arc::new(Served(Mutex::new(Some(b"origin bytes".to_vec()))));
        Ok(super::FetchResponse { status: 200, headers: HashMap::from([("Content-Type".to_string(), "image/jpeg".to_string())]), body })
    }
}

fn images(names: &[&str]) -> Vec<MediaItem> {
    names.iter().map(|name| MediaItem::bare(&format!("https://o.test/{name}.jpg"), crate::api::Kind::Image)).collect()
}

#[test]
fn inv_window_01_a_video_window_is_ready_by_the_first_frame_or_the_cache_and_a_page_change_resets_it() {
    let dir = tempfile::tempdir().unwrap();
    let caller = caller();
    let factory: Arc<MockFactory> = Arc::default();
    let (layer, cache) = caller.block_on(async {
        let parts = crate::api::assemble(assembly(dir.path(), Arc::new(MockFetcher::default()), factory.clone())).await.unwrap();
        let cache = parts.cache.clone();
        (MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new()), cache)
    });
    let (framed, bare, cached) = ("https://o.test/framed.mp4", "https://o.test/bare.mp4", "https://o.test/cached.mp4");
    caller.block_on(cache.put("https://o.test/cached.mp4#0", crate::cache::Kind::Stream, b"first block")).unwrap();
    let seen = Arc::new(ReadySeen::default());
    let window = ring(&layer, 0, [framed, bare, cached].map(|url| Some(MediaItem::bare_video(url))).to_vec());
    window.subscribe_ready(seen.clone()).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(framed).is_some()));
    caller.block_on(async { tokio::time::sleep(SETTLE).await });
    assert!(!seen.is_ready());

    let (index, slot) = factory.slot_holding(framed).unwrap();
    layer.player_events().visual_ready(index, slot.generation());
    caller.block_on(eventually(|| seen.is_ready()));
    window.move_to(1).unwrap();
    caller.block_on(eventually(|| !seen.is_ready()));
    window.move_to(2).unwrap();

    caller.block_on(eventually(|| seen.is_ready()));
}

#[test]
fn inv_window_02_an_image_window_is_ready_when_every_source_of_the_active_page_is_cached_or_failed_and_a_move_resets_it() {
    let dir = tempfile::tempdir().unwrap();
    let origin = Arc::new(Gated::default());
    let layer = MediaLayer::open(config(dir.path(), origin.clone(), shared::<FakeSettings>(), None)).unwrap();
    let caller = caller();
    let seen = Arc::new(ReadySeen::default());
    let grid = layer.open_image_window("grid".to_string(), 1, false).unwrap();
    grid.subscribe_ready(seen.clone()).unwrap();
    grid.replace_rows(vec![images(&["a"])]).unwrap();

    caller.block_on(eventually_async(|| layer.is_cached(images(&["a"]).remove(0))));
    grid.replace_rows(vec![images(&["a", "slow-a"])]).unwrap();
    caller.block_on(async { tokio::time::sleep(SETTLE).await });
    assert!(!seen.is_ready(), "ready while a source of the page was still loading");
    origin.release("slow-a");
    caller.block_on(eventually(|| seen.is_ready()));
    grid.replace_rows(vec![images(&["a", "slow-a"]), images(&["slow-b"])]).unwrap();
    grid.show_rows(1, 1).unwrap();
    caller.block_on(eventually(|| !seen.is_ready()));
    origin.release("slow-b");
    caller.block_on(eventually(|| seen.is_ready()));

    let quiet_dir = tempfile::tempdir().unwrap();
    let quiet_origin = Arc::new(Gated::default());
    let quiet = MediaLayer::open(config(quiet_dir.path(), quiet_origin.clone(), shared::<FakeSettings>(), None)).unwrap();
    quiet.player_events().background();
    caller.block_on(eventually_async(|| async { matches!(quiet.controlled(|assembled| async move { assembled.stage.prefetch_suspended() }).await, Ok(true)) }));
    let fetched = Arc::new(ReadySeen::default());
    let late = quiet.open_image_window("grid".to_string(), 1, false).unwrap();
    late.subscribe_ready(fetched.clone()).unwrap();
    late.replace_rows(vec![images(&["slow-c"])]).unwrap();
    quiet_origin.release("slow-c");
    caller.block_on(quiet.fetch_image("https://o.test/slow-c.jpg".to_string(), HashMap::new())).unwrap();
    caller.block_on(eventually(|| fetched.is_ready()));

    let offline_dir = tempfile::tempdir().unwrap();
    let offline = MediaLayer::open(config(offline_dir.path(), Arc::new(Offline), shared::<FakeSettings>(), None)).unwrap();
    let failed = Arc::new(ReadySeen::default());
    let broken = offline.open_image_window("grid".to_string(), 1, false).unwrap();
    broken.subscribe_ready(failed.clone()).unwrap();
    broken.replace_rows(vec![images(&["x"])]).unwrap();
    caller.block_on(eventually(|| failed.is_ready()));
}

#[derive(Default)]
struct Hanging {
    fetched: Mutex<Vec<String>>,
    released: Arc<std::sync::atomic::AtomicUsize>,
}

struct Endless(Arc<std::sync::atomic::AtomicUsize>);

#[async_trait]
impl BodyReader for Endless {
    async fn next_chunk(&self) -> Result<Option<Vec<u8>>, MediaError> {
        std::future::pending().await
    }

    fn release(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait]
impl Fetcher for Hanging {
    async fn fetch(&self, request: super::FetchRequest) -> Result<super::FetchResponse, MediaError> {
        self.fetched.lock().unwrap().push(request.url);
        Ok(super::FetchResponse { status: 200, headers: HashMap::new(), body: Arc::new(Endless(self.released.clone())) })
    }
}

impl Hanging {
    fn fetches(&self, name: &str) -> usize {
        self.fetched.lock().unwrap().iter().filter(|url| url.contains(name)).count()
    }

    fn is_released(&self) -> bool {
        self.released.load(std::sync::atomic::Ordering::SeqCst) > 0
    }
}

fn whole_photo(name: &str) -> MediaItem {
    MediaItem { strategy: crate::prefetch::CacheStrategy::Whole, ..MediaItem::bare(&format!("https://o.test/{name}"), crate::api::Kind::Image) }
}

fn hanging_layer(dir: &std::path::Path) -> (Arc<MediaLayer>, Arc<Hanging>) {
    let origin = Arc::new(Hanging::default());
    (MediaLayer::open(config(dir, origin.clone(), shared::<FakeSettings>(), None)).unwrap(), origin)
}

#[test]
fn inv_strategy_16_a_declaration_made_before_the_assembly_downloads_after_it_without_any_place_or_window() {
    let dir = tempfile::tempdir().unwrap();
    slow_assembly(dir.path(), SLOW);
    let origin = Arc::new(Origin::default());
    let layer = MediaLayer::open(config(dir.path(), origin.clone(), shared::<FakeSettings>(), None)).unwrap();

    let declaration = layer.open_whole_declaration().unwrap();
    let windowed = MediaItem::bare("https://o.test/c.jpg", crate::api::Kind::Image);
    declaration.add(vec![whole_photo("a.jpg"), whole_photo("b.jpg"), windowed]).unwrap();

    caller().block_on(eventually(|| {
        let seen = origin.seen.lock().unwrap();
        ["a.jpg", "b.jpg", "c.jpg"].iter().all(|name| seen.iter().any(|request| request.url.contains(name)))
    }));
}

#[test]
fn inv_strategy_19_a_declaration_ignores_sources_that_are_not_remote() {
    let dir = tempfile::tempdir().unwrap();
    let origin = Arc::new(Origin::default());
    let layer = MediaLayer::open(config(dir.path(), origin.clone(), shared::<FakeSettings>(), None)).unwrap();
    let declaration = layer.open_whole_declaration().unwrap();
    let ignored = ["file:///bundle/local.jpg", "", "ftp://o.test/ftp.jpg", "https://", "o.test/bare.jpg"]
        .map(|url| MediaItem { url: url.to_string(), ..whole_photo("x") });

    declaration.add(ignored.into_iter().chain([whole_photo("remote.jpg")]).collect()).unwrap();

    let caller = caller();
    caller.block_on(eventually(|| origin.seen.lock().unwrap().iter().any(|request| request.url.ends_with("remote.jpg"))));
    caller.block_on(layer.fetch_image("https://o.test/after.jpg".to_string(), HashMap::new())).unwrap();
    let seen: Vec<String> = origin.seen.lock().unwrap().iter().map(|request| request.url.clone()).collect();
    assert_eq!(seen.iter().filter(|url| !url.ends_with("remote.jpg") && !url.ends_with("after.jpg")).count(), 0, "{seen:?}");
}

#[test]
fn inv_strategy_18_closing_the_declaration_cancels_a_load_nobody_else_holds() {
    let dir = tempfile::tempdir().unwrap();
    let (layer, origin) = hanging_layer(dir.path());
    let first = layer.open_whole_declaration().unwrap();
    let second = layer.open_whole_declaration().unwrap();
    first.add(vec![whole_photo("held.jpg")]).unwrap();
    second.add(vec![whole_photo("held.jpg")]).unwrap();
    let caller = caller();
    caller.block_on(eventually(|| origin.fetches("held.jpg") == 1));

    first.release();
    caller.block_on(layer.stats()).unwrap();
    assert!(!origin.is_released(), "a source another declaration holds was cancelled");

    second.release();
    caller.block_on(eventually(|| origin.is_released()));
    assert!(matches!(second.add(vec![whole_photo("late.jpg")]), Err(MediaError::Closed)));
}

fn decode_layer(dir: &std::path::Path, budget: u64) -> Arc<MediaLayer> {
    let mut open = config(dir, Arc::new(Offline), shared::<FakeSettings>(), None);
    open.defaults = Limits { memory_budget: budget, ..default_limits() };
    MediaLayer::open(open).unwrap()
}

#[test]
fn inv_memory_14_a_decode_waits_for_the_ledger_and_a_released_hold_admits_the_next() {
    let dir = tempfile::tempdir().unwrap();
    let layer = decode_layer(dir.path(), 64 << 20);
    let caller = caller();
    caller.block_on(async {
        let first = layer.reserve_decode(40 << 20).await.unwrap();
        assert_eq!(first.bytes(), 40 << 20);
        let mut second = Box::pin(layer.reserve_decode(40 << 20));
        assert!(pending_once(async { let _ = second.as_mut().await; }).await, "a second decode got bytes the ledger did not have");
        first.release();
        assert_eq!(first.bytes(), 0);
        assert_eq!(second.await.unwrap().bytes(), 40 << 20);
    });
}

#[test]
fn inv_memory_14_a_decode_above_the_ceiling_is_refused_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let layer = decode_layer(dir.path(), 64 << 20);
    let refused = caller().block_on(layer.reserve_decode(1 << 30));
    assert!(matches!(refused, Err(MediaError::Overloaded { .. })));
}

#[test]
fn inv_memory_15_the_decode_cost_is_one_rgba_raster_and_refuses_empty_and_oversized_sides() {
    assert_eq!(crate::ffi::decode_cost(100, 50), Some(20_000));
    assert_eq!(crate::ffi::decode_cost(16_384, 16_384), Some(16_384 * 16_384 * 4));
    assert_eq!(crate::ffi::decode_cost(16_385, 1), None);
    assert_eq!(crate::ffi::decode_cost(0, 10), None);
}

#[test]
fn inv_capture_13_a_session_hold_shares_the_budget_with_decodes_and_a_release_admits_the_next() {
    let dir = tempfile::tempdir().unwrap();
    let layer = decode_layer(dir.path(), 64 << 20);
    caller().block_on(async {
        let session = layer.reserve_capture(40 << 20).await.unwrap();
        assert_eq!(session.bytes(), 40 << 20);
        let mut decode = Box::pin(layer.reserve_decode(40 << 20));
        assert!(pending_once(async { let _ = decode.as_mut().await; }).await, "a decode got bytes the capture session holds");
        session.release();
        assert_eq!(decode.await.unwrap().bytes(), 40 << 20);
        assert!(matches!(layer.reserve_capture(1 << 30).await, Err(MediaError::Overloaded { .. })));
    });
}
