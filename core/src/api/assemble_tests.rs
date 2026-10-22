use super::test_support::{assembly, shared, FakeSettings, MockFactory};
use super::*;
use crate::cache::{self, Cache};
use crate::fetch::mock::{MockFetcher, Script};
use crate::identity::cache_key;
use crate::limits::Limits;
use crate::memory::DeviceMemory;
use crate::pool::{Command, Ring};
use crate::spec::PoolSpec;
use std::sync::Arc;

const MIB: u64 = 1024 * 1024;

#[tokio::test]
async fn inv_cache_26_saved_budget_applies_before_startup_eviction() {
    let dir = tempfile::tempdir().unwrap();
    let saved = Limits { budget: 100, min_budget: 1, ..Limits::default() };
    let cache = Cache::open(dir.path(), saved, u64::MAX);
    cache.ready().await;
    for key in ["a", "b", "c"] {
        cache.put(key, cache::Kind::Image, &[1; 30]).await.unwrap();
    }
    cache.flush().await.unwrap();
    let settings = shared::<FakeSettings>();
    settings.values.lock().unwrap().insert(super::controls::LIMITS_KEY.into(), saved.encode());
    let mut config = assembly(dir.path(), Arc::new(MockFetcher::new()), shared::<MockFactory>());
    config.settings = settings;
    config.defaults = Limits { budget: 60, ..saved };
    let parts = assemble(config).await.unwrap();
    parts.cache.ready().await;
    parts.cache.evict_now().await;
    assert_eq!(parts.cache.stats().await.total_bytes, 90);
}

#[test]
fn inv_api_09_ceiling_is_the_smaller_of_the_option_and_the_device_share() {
    let low = DeviceMemory { total_mb: 100, heap_class_mb: None, low_ram: true };
    let big = DeviceMemory { total_mb: 8192, heap_class_mb: None, low_ram: false };

    assert_eq!(ledger_for(&Limits::default(), low).ceiling(), 15 * MIB);
    assert_eq!(ledger_for(&Limits::default(), big).ceiling(), 256 * MIB);
}

#[test]
fn inv_api_09_the_default_memory_holds_the_default_pool() {
    let device = DeviceMemory { total_mb: 8192, heap_class_mb: None, low_ram: false };
    let limits = Limits::default();
    let mut ring = Ring::with_ledger(pool_config(&limits, device, &PoolSettings::default()), ledger_for(&limits, device));
    let keys: Vec<Option<String>> = (0..3).map(|index| Some(format!("k{index}"))).collect();

    let loads = ring.plan(0, 1, None, &keys).iter().filter(|command| matches!(command, Command::Load { .. })).count();

    assert_eq!(loads, 3);
}

#[tokio::test]
async fn inv_api_09_purge_removes_the_entry_of_the_url_and_its_ranged_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let url = "https://cdn.test/a.mp4";
    let cache = Cache::open(dir.path(), Limits::default(), u64::MAX);
    for key in [cache_key(url), format!("{}#0", cache_key(url)), format!("{}#1", cache_key(url)), "https://cdn.test/b.mp4".to_string()] {
        cache.put(&key, cache::Kind::Stream, b"bytes").await.unwrap();
    }

    CacheControl::purge(&cache, url).await;

    assert_eq!(cache.stats().await.file_count, 1);
    assert!(cache.contains("https://cdn.test/b.mp4").await);
}

#[tokio::test]
async fn inv_limits_01_invalid_defaults_fail_the_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>());
    config.settings = shared::<FakeSettings>();
    config.defaults = Limits { budget: 1, ..Limits::default() };

    let result = assemble(config).await;

    assert!(matches!(result, Err(MediaError::InvalidLimits { .. })));
}

#[tokio::test]
async fn inv_api_09_assembling_wires_the_fetcher_through_the_proxy_and_prefetch_into_the_disk_cache() {
    let dir = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(MockFetcher::new());
    let url = "https://o.test/clip.mp4";
    fetcher.route(url, Script::ok(vec![9; 3 * 1024 * 1024]));

    let parts = assemble(assembly(dir.path(), fetcher.clone(), shared::<MockFactory>())).await.unwrap();
    parts.prefetcher.set_window(0, vec![None, None, Some(super::test_support::video(url).for_prefetch())]);

    super::test_support::eventually_async(|| async { parts.cache.contains(&format!("{url}#0")).await }).await;
    assert_eq!(parts.proxy.readiness(), crate::proxy::Readiness::Ready);
    assert_eq!(fetcher.requests().iter().map(|request| request.range).collect::<Vec<_>>(), vec![Some((0, Some(MIB - 1)))]);
}

#[tokio::test]
async fn inv_api_09_the_proxy_downloads_wait_on_the_same_ledger_as_the_pool() {
    let dir = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(MockFetcher::new());
    let url = "https://o.test/clip.mp4";
    fetcher.route(url, Script::ok(vec![9; 3 * 1024 * 1024]));
    let parts = assemble(assembly(dir.path(), fetcher.clone(), shared::<MockFactory>())).await.unwrap();
    let everything = parts.ledger.ceiling() - parts.ledger.used();
    let held = parts.ledger.reserve(crate::memory::Owner::Queue, everything).unwrap();

    parts.prefetcher.set_window(0, vec![None, None, Some(super::test_support::video(url).for_prefetch())]);
    super::test_support::eventually(|| parts.ledger.waiting() == 1).await;
    assert_eq!(parts.cache.stats().await.file_count, 0);
    drop(held);

    super::test_support::eventually_async(|| async { parts.cache.contains(&format!("{url}#0")).await }).await;
    assert_eq!(parts.ledger.used(), 0);
}

#[tokio::test]
async fn inv_api_09_the_namespace_headers_of_the_assembly_reach_the_cache_key() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>());
    config.namespace_headers = vec!["Authorization".to_string()];
    let parts = assemble(config).await.unwrap();
    let as_user = |token: &str| std::collections::HashMap::from([("Authorization".to_string(), token.to_string())]);

    let first = parts.proxy.key_for("https://o.test/a.mp4", &as_user("a"));

    assert_ne!(first, parts.proxy.key_for("https://o.test/a.mp4", &as_user("b")));
}

#[tokio::test]
async fn inv_api_09_the_device_profile_and_the_reach_setting_shape_the_pool() {
    let dir = tempfile::tempdir().unwrap();
    let build = |device: DeviceMemory, reach: usize| {
        let mut config = assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>());
        config.device = device;
        config.pool = PoolSettings { reach, ..PoolSettings::default() };
        config
    };
    let strong = DeviceMemory { total_mb: 8192, heap_class_mb: None, low_ram: false };
    let weak = DeviceMemory { total_mb: 1024, heap_class_mb: None, low_ram: true };

    let strong_slots = assemble(build(strong, 1)).await.unwrap().stage.slot_count();
    let wide_slots = assemble(build(strong, 2)).await.unwrap().stage.slot_count();
    let weak_slots = assemble(build(weak, 1)).await.unwrap().stage.slot_count();

    assert_eq!((strong_slots, wide_slots, weak_slots), (3, 5, 2));
}

#[test]
fn inv_api_09_pool_settings_take_reach_and_timings_from_the_spec() {
    let spec = PoolSpec { slots: 4, reach: 2, hold_off: 5, trim_delay: 60, low_ram_slots: Some(3), low_ram_reach: Some(1) };

    let settings = PoolSettings::from(&spec);

    assert_eq!((settings.reach, settings.hold_off_ms, settings.trim_delay_ms), (2, 5_000, 60_000));
    assert_eq!((settings.profile.low_slots, settings.profile.low_reach), (3, 1));
    assert_eq!((PoolSettings::default().profile.low_slots, PoolSettings::default().profile.low_reach), (2, 0));
}

#[tokio::test]
async fn inv_prefetch_10_assembling_with_image_windows_gives_each_name_its_range() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = assembly(dir.path(), Arc::new(MockFetcher::new()), shared::<MockFactory>());
    config.image_windows = vec![("avatar".to_string(), crate::prefetch::PageWindow { behind: 0, from: 1, to: 2 })];
    let parts = assemble(config).await.unwrap();
    let pages = |count: usize| -> Vec<Vec<MediaItem>> {
        (0..count).map(|index| vec![MediaItem::bare(&format!("https://o.test/p{index}.jpg"), Kind::Image)]).collect()
    };

    parts.stage.replace_image_window("avatar", 0, pages(5));
    assert_eq!(parts.prefetcher.counters().requested, 1);
    parts.stage.replace_image_window("grid", 0, pages(5));

    assert_eq!(parts.prefetcher.counters().requested, 1 + 5);
}
