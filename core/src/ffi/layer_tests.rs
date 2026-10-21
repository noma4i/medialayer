use super::*;
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
}

struct Totals(Mutex<Vec<u64>>);

impl StatsListener for Totals {
    fn changed(&self, stats: MediaStats) {
        self.0.lock().unwrap().push(stats.total_bytes);
    }
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
    }
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
        assert_eq!(bytes, b"origin bytes");
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
        cache.put("small", kind, &[1; 2]).await.unwrap();
        cache.put("large", kind, &[2; 7]).await.unwrap();
        cache.put("new", kind, &[3; 4]).await.unwrap();
        assert!(cache.contains("small").await, "{section} evicted the oldest small entry");
        assert!(!cache.contains("large").await);
        assert!(cache.contains("new").await);
    }
}

#[tokio::test]
async fn inv_ffi_02_opening_reads_nothing_until_the_first_call_awaits() {
    let settings: Arc<FakeSettings> = shared();
    let dir = tempfile::tempdir().unwrap();
    let mut assembled = assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>());
    assembled.settings = settings.clone();
    let parts = crate::api::assemble(assembled).await.unwrap();

    let layer = MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new());

    assert_eq!(settings.reads.load(std::sync::atomic::Ordering::SeqCst), 0);
    layer.limits().await;
    assert_eq!(settings.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
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
        (layer.limits().await, layer.stats().await)
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
            if layer.stats().await.file_count > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        layer.stats().await.file_count
    });
    caller.block_on(layer.purge(source.url.clone()));

    assert_eq!(reply.body, b"origin bytes");
    assert_eq!(origin.seen.lock().unwrap()[0].headers.get("Authorization").map(String::as_str), Some("token"));
    assert_eq!(stored, 1);
    assert_eq!(caller.block_on(layer.stats()).file_count, 0);
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
    let _subscription = layer.subscribe_state(url.to_string(), seen.clone());
    let caller = caller();

    layer.replace_window(1, (0..3).map(|index| Some(MediaItem::bare_video(&format!("https://o.test/v{index}.mp4")))).collect());
    layer.set_playing(url.to_string(), true);
    layer.set_muted_all(true);
    layer.visible(MediaItem::bare_video(url), true);
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))));
    let (index, slot): (u32, Arc<MockSlot>) = factory.slot_holding(url).unwrap();
    layer.player_events().ready(index);

    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Ready)));
    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Mute)));
    layer.release_all();
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
    let _subscription = layer.subscribe_stats(totals.clone());

    caller.block_on(async {
        cache.put("https://o.test/a.jpg", crate::cache::Kind::Image, &[1; 500]).await.unwrap();
        eventually_async(|| async { totals.0.lock().unwrap().last() == Some(&500) }).await;
    });
}

#[derive(Default)]
struct CountingReleaser(std::sync::atomic::AtomicUsize);

impl MemoryReleaser for CountingReleaser {
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

    assert_eq!((first, second), (Ok(b"origin bytes".to_vec()), Ok(b"origin bytes".to_vec())));
    assert_eq!(together, (Ok(b"origin bytes".to_vec()), Ok(b"origin bytes".to_vec())));
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
fn inv_placeholder_02_a_source_is_cached_by_its_first_block_and_the_placeholder_follows_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let caller = caller();
    let (layer, cache) = caller.block_on(async {
        let parts = crate::api::assemble(assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>())).await.unwrap();
        let cache = parts.cache.clone();
        (MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new()), cache)
    });
    let cached = MediaItem::bare_video("https://o.test/cached.mp4");
    let missing = MediaItem::bare_video("https://o.test/missing.mp4");

    caller.block_on(cache.put("https://o.test/cached.mp4#0", crate::cache::Kind::Stream, b"first block")).unwrap();

    assert!(caller.block_on(layer.is_cached(cached.clone())));
    assert!(!caller.block_on(layer.is_cached(missing.clone())));
    assert_eq!(caller.block_on(layer.placeholder(cached, false)), crate::api::Placeholder::None);
    assert_eq!(caller.block_on(layer.placeholder(missing, false)), crate::api::Placeholder::FirstFrame);
}

#[test]
fn inv_ffi_03_an_item_state_from_a_native_loader_reaches_the_state_subscribers_and_a_seek_reaches_the_player() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), Some(slots))).unwrap();
    let seen = Arc::new(Seen(Mutex::new(Vec::new())));
    let _subscription = layer.subscribe_state("https://o.test/a.jpg".to_string(), seen.clone());
    let caller = caller();

    layer.item_state("https://o.test/a.jpg".to_string(), MediaState::Loading);
    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Loading)));
    layer.item_state("https://o.test/a.jpg".to_string(), MediaState::Ready);
    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Ready)));

    let url = "https://o.test/v1.mp4";
    layer.replace_window(0, vec![Some(MediaItem::bare_video(url))]);
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    layer.seek(url.to_string(), 1_500);
    layer.player_events().attach(url.to_string(), 5);
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
    let limits = caller.block_on(layer.limits());

    assert_eq!((limits.budget, limits.slots), (3_000_000_000, 5));
    assert!(settings.writes.lock().unwrap().is_empty());
    let saved = Limits { budget: 600_000_000, ..parsed.defaults.clone() };
    caller.block_on(layer.set_limits(saved.clone())).unwrap();
    drop(layer);
    let second_dir = tempfile::tempdir().unwrap();
    assert_eq!(caller.block_on(from_spec(second_dir.path()).limits()), saved);
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

    layer.replace_image_window("grid".to_string(), 0, poster_pages(1));
    caller.block_on(eventually(|| !origin.seen.lock().unwrap().is_empty()));
    let fetched = caller.block_on(layer.fetch_image("https://o.test/p0.jpg".to_string(), HashMap::new()));

    assert_eq!(fetched, Ok(b"origin bytes".to_vec()));
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
    let _subscription = layer.subscribe_state(url.to_string(), seen.clone());
    let caller = caller();

    layer.replace_window(0, vec![Some(MediaItem::bare_video(url))]);
    layer.set_playing(url.to_string(), true);
    layer.player_events().attach(url.to_string(), 5);
    let declared_in = started.elapsed();
    let limits = caller.block_on(layer.limits());

    assert!(declared_in < SLOW / 2, "the declarations waited {declared_in:?} for the assembly");
    assert!(started.elapsed() >= SLOW, "the async call returned before the assembly ended");
    assert_eq!(limits, default_limits());
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play) && slot.calls().contains(&SlotCall::Attach { view_token: 5 }))));
    let (index, _) = factory.slot_holding(url).unwrap();
    layer.player_events().ready(index);
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
        assembled.stage.set_playing(&url, true);
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
    let subscribed = assembled.controls.wakes();
    assert_eq!(subscribed, 1, "the subscription delivers the current stats once");

    cache.put("https://o.test/one.jpg", crate::cache::Kind::Image, &[1; 10]).await.unwrap();
    elapse(std::time::Duration::from_secs(1)).await;
    assert_eq!(assembled.controls.wakes() - subscribed, 1, "one write, one publication");

    for index in 0..100 {
        cache.put(&format!("https://o.test/burst{index}.jpg"), crate::cache::Kind::Image, &[1; 10]).await.unwrap();
    }
    elapse(std::time::Duration::from_secs(1)).await;
    assert!(assembled.controls.wakes() - subscribed <= 3, "a burst of 100 writes merges: {}", assembled.controls.wakes() - subscribed);
    assert_eq!(totals.0.lock().unwrap().last(), Some(&(301 * 10)));
}

fn failed_layer(runtime: tokio::runtime::Runtime) -> Arc<MediaLayer> {
    MediaLayer::assembling(runtime, async { Err(MediaError::Unexpected { detail: "loopback bind failed".to_string() }) }, None, Vec::new())
}

#[test]
fn inv_ffi_04_a_layer_that_failed_to_assemble_answers_the_non_throwing_calls_with_empty_values() {
    let layer = failed_layer(tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap());
    let item = crate::api::test_support::video("https://o.test/a.mp4");

    caller().block_on(async {
        assert_eq!(layer.stats().await, MediaStats::default());
        assert_eq!(layer.limits().await, default_limits());
        assert!(!layer.is_cached(item.clone()).await);
        layer.clear(None).await;
        layer.purge(item.url.clone()).await;
        layer.evict_now().await;
        assert!(matches!(layer.play_url(item).await, Err(MediaError::Unexpected { .. })));
    });
}

#[test]
fn inv_ffi_04_a_panicking_media_task_is_an_error_not_a_panic_across_the_bridge() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared(), None)).unwrap();

    let outcome = caller().block_on(layer.with_assembled(|_| async { panic!("a media task failed") }));

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
        layer.with_assembled(|_| async move {
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
