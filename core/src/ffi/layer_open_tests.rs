use super::*;
use super::layer_harness::*;
use super::ingress::PlaceInput;
use crate::api::test_support::{assembly, eventually, eventually_async, shared, FakeSettings, MockFactory, MockSlot};
use crate::api::{MediaItem, MediaState, PlayerSlotFactory, SlotCall, LIMITS_KEY};
use crate::fetch::mock::MockFetcher;
use crate::proxy::test_client;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;
use url::Url;

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
        eventually_async(|| async { layer.stats().await.unwrap().file_count > 0 }).await;
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
        crate::test_signal::fire();
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

    caller().block_on(eventually(|| releaser.0.load(std::sync::atomic::Ordering::SeqCst) == 1));
    assert_eq!(releaser.0.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn inv_pool_09_background_unloads_every_platform_slot_including_the_playing_one_and_drops_the_image_memory_cache() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let releaser: Arc<CountingReleaser> = shared();
    let mut open = config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), Some(slots));
    open.memory_releaser = Some(releaser.clone());
    let layer = MediaLayer::open(open).unwrap();
    let urls: Vec<String> = (0..3).map(|index| format!("https://o.test/v{index}.mp4")).collect();
    let caller = caller();
    let _ring = ring(&layer, 1, urls.iter().map(|url| Some(MediaItem::bare_video(url))).collect());
    let _place = place(&layer, &urls[1], PlaceInput { playing: true, visible: true, ..PlaceInput::default() });
    caller.block_on(eventually(|| factory.slot_holding(&urls[1]).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))));

    layer.player_events().background();

    let drops = || releaser.0.load(std::sync::atomic::Ordering::SeqCst);
    caller.block_on(eventually(|| drops() == 1 && urls.iter().all(|url| factory.slot_holding(url).is_none())));
}
