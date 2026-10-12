use super::test_support::{eventually, pool_stats, shared, FakeCache, FakePool, FakePrefetch, FakeRelease, FakeSettings, FakeTransport};
use super::*;
use crate::cache;
use crate::limits::Limits;
use crate::memory::{effective_ceiling, DeviceMemory, Ledger, ShareTuning};
use crate::pool::Released;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

const MIB: u64 = 1024 * 1024;

const DEVICE: DeviceMemory = DeviceMemory { total_mb: 8192, heap_class_mb: None, low_ram: false };

struct Rig {
    cache: Arc<FakeCache>,
    pool: Arc<FakePool>,
    prefetch: Arc<FakePrefetch>,
    transport: Arc<FakeTransport>,
    settings: Arc<FakeSettings>,
    release: Arc<FakeRelease>,
    ledger: Ledger,
    controls: Controls,
}

fn rig_with_saved(saved: Option<&str>) -> Rig {
    let settings: Arc<FakeSettings> = shared();
    if let Some(saved) = saved {
        settings.values.lock().unwrap().insert(LIMITS_KEY.to_string(), saved.to_string());
    }
    rig_on(settings)
}

fn rig_on(settings: Arc<FakeSettings>) -> Rig {
    let cache: Arc<FakeCache> = shared();
    let pool: Arc<FakePool> = shared();
    let prefetch: Arc<FakePrefetch> = shared();
    let transport: Arc<FakeTransport> = shared();
    let release: Arc<FakeRelease> = shared();
    let ledger = Ledger::new(effective_ceiling(Limits::default().memory_budget, DEVICE, ShareTuning::default()));
    let ports = ControlPorts {
        cache: cache.clone(),
        pool: pool.clone(),
        prefetch: prefetch.clone(),
        settings: settings.clone(),
        ledger: ledger.clone(),
        device: DEVICE,
        transport: transport.clone(),
        pool_settings: PoolSettings::default(),
        release: release.clone(),
    };
    let controls = Controls::new(Handle::current(), ports, Limits::default(), crate::change::Change::new());
    Rig { cache, pool, prefetch, transport, settings, release, ledger, controls }
}

fn smaller_budget() -> Limits {
    Limits { budget: 512 * MIB, ..Limits::default() }
}

#[tokio::test]
async fn inv_limits_03_saved_limits_win_over_defaults_and_reach_the_cache() {
    let rig = rig_with_saved(Some("budget=536870912"));

    assert_eq!(rig.controls.limits().await.budget, 512 * MIB);
    assert_eq!(*rig.cache.applied.lock().unwrap(), vec![smaller_budget()]);
}

#[tokio::test]
async fn inv_limits_03_invalid_saved_limits_fall_back_to_defaults() {
    let rig = rig_with_saved(Some("budget=1024"));

    assert_eq!(rig.controls.limits().await, Limits::default());
    assert!(rig.cache.applied.lock().unwrap().is_empty());
}

#[tokio::test]
async fn inv_limits_04_set_limits_applies_to_the_cache_and_survives_a_reopen() {
    let rig = rig_with_saved(None);

    rig.controls.set_limits(smaller_budget()).await.unwrap();

    assert_eq!(*rig.cache.applied.lock().unwrap(), vec![smaller_budget()]);
    assert_eq!(rig.settings.writes.lock().unwrap().len(), 1);
    let reopened = rig_on(rig.settings.clone());
    assert_eq!(reopened.controls.limits().await, smaller_budget());
}

#[tokio::test]
async fn inv_limits_01_invalid_limits_are_refused_without_side_effects() {
    let rig = rig_with_saved(None);
    let invalid = Limits { budget: 1, ..Limits::default() };

    let result = rig.controls.set_limits(invalid).await;

    assert!(matches!(result, Err(MediaError::InvalidLimits { .. })));
    assert!(rig.settings.writes.lock().unwrap().is_empty());
    assert!(rig.cache.applied.lock().unwrap().is_empty());
    assert_eq!(rig.controls.limits().await, Limits::default());
}

#[tokio::test]
async fn inv_limits_04_reset_limits_drops_the_saved_value_and_restores_defaults() {
    let rig = rig_with_saved(Some("budget=536870912"));
    rig.controls.limits().await;

    rig.controls.reset_limits().await.unwrap();

    assert_eq!(*rig.settings.removed.lock().unwrap(), vec![LIMITS_KEY.to_string()]);
    assert_eq!(rig.cache.applied.lock().unwrap().last(), Some(&Limits::default()));
    assert_eq!(rig.controls.limits().await, Limits::default());
}

#[tokio::test]
async fn inv_api_07_stats_join_the_cache_and_the_pool() {
    let rig = rig_with_saved(None);
    {
        let mut stats = rig.cache.stats.lock().unwrap();
        stats.total_bytes = 300;
        stats.file_count = 3;
        stats.evictions = 4;
        stats.max_hold = std::time::Duration::from_millis(9);
        stats.per_kind.insert(cache::Kind::Stream, cache::KindStats { bytes: 250, files: 2 });
        stats.per_kind.insert(cache::Kind::Image, cache::KindStats { bytes: 50, files: 1 });
    }
    *rig.pool.stats.lock().unwrap() =
        Some(crate::pool::PoolStats { memory_bytes: 77, released: Released { pressure: 5, ..Released::default() }, ..pool_stats() });

    let stats = rig.controls.stats().await;

    assert_eq!((stats.total_bytes, stats.file_count, stats.evictions, stats.max_hold_ms), (300, 3, 4, 9));
    assert_eq!(
        stats.kinds.iter().map(|usage| (usage.kind, usage.bytes, usage.files)).collect::<Vec<_>>(),
        vec![(Kind::Image, 50, 1), (Kind::Video, 250, 2)]
    );
    assert_eq!((stats.slots.hot, stats.slots.warm, stats.slots.cold, stats.memory_bytes, stats.released.pressure), (1, 2, 3, 77, 5));
}

#[tokio::test]
async fn inv_api_07_clear_purge_and_evict_now_go_to_the_cache() {
    let rig = rig_with_saved(None);

    rig.controls.clear(Some(Kind::Video)).await;
    rig.controls.clear(None).await;
    rig.controls.purge("https://cdn.test/a.mp4").await;
    rig.controls.evict_now().await;

    assert_eq!(*rig.cache.cleared.lock().unwrap(), vec![Some(cache::Kind::Stream), None]);
    assert_eq!(*rig.cache.purged.lock().unwrap(), vec!["https://cdn.test/a.mp4".to_string()]);
    assert_eq!(rig.cache.evictions.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn inv_image_03_clearing_everything_or_the_images_drops_the_decoder_memory_and_another_kind_does_not() {
    let rig = rig_with_saved(None);
    let drops = || rig.release.drops.load(Ordering::SeqCst);

    rig.controls.clear(Some(Kind::Video)).await;
    rig.controls.clear(Some(Kind::Audio)).await;
    assert_eq!(drops(), 0);

    rig.controls.clear(Some(Kind::Image)).await;
    assert_eq!(drops(), 1);
    rig.controls.clear(None).await;
    assert_eq!(drops(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv_api_07_changing_limits_notifies_stats_subscribers() {
    let rig = rig_with_saved(None);
    rig.cache.stats.lock().unwrap().total_bytes = 42;
    let seen: Arc<Mutex<Vec<u64>>> = Arc::default();
    let sink = seen.clone();
    let _subscription = rig.controls.subscribe_stats(Arc::new(move |stats: MediaStats| sink.lock().unwrap().push(stats.total_bytes)));

    rig.controls.set_limits(smaller_budget()).await.unwrap();

    eventually(|| seen.lock().unwrap().last() == Some(&42)).await;
}

#[tokio::test]
async fn inv_limits_04_new_limits_move_the_memory_ceiling_and_reach_the_prefetcher_and_a_reset_brings_them_back() {
    let rig = rig_with_saved(None);
    let before = rig.ledger.ceiling();
    let smaller = Limits { memory_budget: 32 * MIB, prefetch_to: 7, ..Limits::default() };

    rig.controls.set_limits(smaller).await.unwrap();

    assert_eq!(rig.ledger.ceiling(), 32 * MIB);
    assert_eq!(rig.prefetch.limits.lock().unwrap().last(), Some(&smaller));
    rig.controls.reset_limits().await.unwrap();
    assert_eq!(rig.ledger.ceiling(), before);
    assert_eq!(rig.prefetch.limits.lock().unwrap().last(), Some(&Limits::default()));
}

#[tokio::test]
async fn inv_limits_03_saved_limits_reach_the_ceiling_and_the_prefetcher_when_they_are_first_read() {
    let rig = rig_with_saved(Some("memory_budget=33554432"));

    rig.controls.limits().await;

    assert_eq!(rig.ledger.ceiling(), 32 * MIB);
    assert_eq!(rig.prefetch.limits.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn inv_limits_03_an_invalid_saved_value_is_dropped_and_named_in_the_diagnostics() {
    let rig = rig_with_saved(Some("budget=1024"));
    assert!(rig.controls.stats().await.diagnostics.is_empty());

    rig.controls.limits().await;

    let diagnostics = rig.controls.stats().await.diagnostics;
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].contains("below min_budget"), "{diagnostics:?}");
    assert_eq!(rig.controls.limits().await, Limits::default());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv_api_07_unchanged_stats_reach_a_subscriber_once_and_a_new_subscriber_gets_the_current_state() {
    let rig = rig_with_saved(None);
    rig.cache.stats.lock().unwrap().total_bytes = 7;
    let first: Arc<Mutex<Vec<u64>>> = Arc::default();
    let sink = first.clone();
    let _first = rig.controls.subscribe_stats(Arc::new(move |stats: MediaStats| sink.lock().unwrap().push(stats.total_bytes)));

    for _ in 0..3 {
        rig.controls.publish_stats().await;
    }
    eventually(|| first.lock().unwrap().len() == 1).await;
    assert_eq!(rig.controls.published_count(), 1);
    assert_eq!(*first.lock().unwrap(), vec![7]);
    let second: Arc<Mutex<Vec<u64>>> = Arc::default();
    let sink = second.clone();
    let _second = rig.controls.subscribe_stats(Arc::new(move |stats: MediaStats| sink.lock().unwrap().push(stats.total_bytes)));
    rig.controls.publish_stats().await;

    eventually(|| second.lock().unwrap().len() == 1).await;
}

#[tokio::test]
async fn inv_limits_04_new_limits_reach_the_live_proxy_and_resize_the_live_pool() {
    let rig = rig_with_saved(None);
    let limits = Limits { slots: 5, timeout_seconds: 7, max_concurrent: 4, ..Limits::default() };

    rig.controls.set_limits(limits).await.unwrap();

    assert_eq!(*rig.transport.limits.lock().unwrap(), vec![limits]);
    assert_eq!(*rig.pool.slots.lock().unwrap(), vec![5]);
}
