use super::*;
use crate::api::test_support::{assembly, eventually, eventually_async, shared, FakeSettings, MockFactory, MockSlot};
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
        dir: dir.to_string_lossy().to_string(),
        fetcher,
        settings,
        slots,
        device: DeviceMemory { total_mb: 4096, heap_class_mb: None, low_ram: false },
        defaults: default_limits(),
        pool: default_pool_settings(),
    }
}

fn caller() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

#[tokio::test]
async fn opening_reads_nothing_until_the_first_call_awaits() {
    let settings: Arc<FakeSettings> = shared();
    let dir = tempfile::tempdir().unwrap();
    let mut assembled = assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>());
    assembled.settings = settings.clone();
    let parts = crate::api::assemble(assembled).await.unwrap();

    let layer = MediaLayer::from_parts(None, Handle::current(), parts);

    assert_eq!(settings.reads.load(std::sync::atomic::Ordering::SeqCst), 0);
    layer.limits().await;
    assert_eq!(settings.reads.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn open_wires_the_real_parts_so_limits_reach_the_store_and_stats_answer() {
    let dir = tempfile::tempdir().unwrap();
    let settings: Arc<FakeSettings> = shared();
    let caller = caller();

    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), settings.clone(), None)).unwrap();
    let smaller = Limits { budget: 512 * 1024 * 1024, ..default_limits() };
    let (limits, stats) = caller.block_on(async {
        layer.set_limits(smaller).await.unwrap();
        (layer.limits().await, layer.stats().await)
    });

    assert_eq!(limits, smaller);
    assert!(settings.writes.lock().unwrap().iter().any(|(key, value)| key == LIMITS_KEY && value.contains("budget=536870912")));
    assert_eq!((stats.total_bytes, stats.file_count), (0, 0));
}

#[test]
fn a_play_url_routes_the_platform_fetcher_through_the_proxy_into_the_disk_cache_and_purge_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let origin = Arc::new(Origin::default());
    let layer = MediaLayer::open(config(dir.path(), origin.clone(), shared::<FakeSettings>(), None)).unwrap();
    let source = MediaItem {
        headers: HashMap::from([("Authorization".to_string(), "token".to_string())]),
        ..MediaItem::bare_video("https://o.test/clip.mp4")
    };
    let caller = caller();

    let route = layer.play_url(source.clone());
    let parsed = Url::parse(&route.url).unwrap();
    let port = parsed.port().unwrap();
    let reply = caller.block_on(test_client::get(port, &format!("{}?{}", parsed.path(), parsed.query().unwrap_or_default())));
    let stored = caller.block_on(async {
        for _ in 0..500 {
            if layer.stats().await.file_count > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        layer.stats().await.file_count
    });
    caller.block_on(layer.purge(source.url.clone()));

    assert_eq!(reply.body, b"origin bytes");
    assert_eq!(origin.seen.lock().unwrap()[0].headers.get("Authorization").map(String::as_str), Some("token"));
    assert_eq!(stored, 1);
    assert_eq!(caller.block_on(layer.stats()).file_count, 0);
    assert_eq!(layer.refresh_route(source, route.clone()), route);
}

#[test]
fn declarations_reach_the_platform_slots_and_the_player_events_move_the_state() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), Some(slots))).unwrap();
    let url = "https://o.test/v1.mp4";
    let seen = Arc::new(Seen(Mutex::new(Vec::new())));
    let _subscription = layer.subscribe_state(url.to_string(), seen.clone());
    let caller = caller();

    layer.set_window(1, (0..3).map(|index| Some(MediaItem::bare_video(&format!("https://o.test/v{index}.mp4")))).collect());
    layer.set_playing(url.to_string(), true);
    layer.set_muted_all(true);
    layer.visible(MediaItem::bare_video(url), true);
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))));
    let (index, slot): (u32, Arc<MockSlot>) = factory.slot_holding(url).unwrap();
    layer.player_events().ready(index);

    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Ready)));
    assert!(slot.calls().contains(&SlotCall::Mute));
    layer.release_all();
    caller.block_on(eventually(|| slot.calls().last() == Some(&SlotCall::Unload)));
}

#[test]
fn a_cache_write_reaches_a_stats_subscriber_without_any_controls_call() {
    let dir = tempfile::tempdir().unwrap();
    let caller = caller();
    let (layer, cache) = caller.block_on(async {
        let mut assembled = assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>());
        assembled.stats_interval = std::time::Duration::from_millis(10);
        let parts = crate::api::assemble(assembled).await.unwrap();
        let cache = parts.cache.clone();
        (MediaLayer::from_parts(None, Handle::current(), parts), cache)
    });
    let totals = Arc::new(Totals(Mutex::new(Vec::new())));
    let _subscription = layer.subscribe_stats(totals.clone());

    caller.block_on(async {
        cache.put("https://o.test/a.jpg", crate::cache::Kind::Image, &[1; 500]).await.unwrap();
        eventually_async(|| async { totals.0.lock().unwrap().last() == Some(&500) }).await;
    });
}
