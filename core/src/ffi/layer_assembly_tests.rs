use super::*;
use super::layer_harness::*;
use super::ingress::{IngressLimits, Pause, PlaceInput};
use crate::api::test_support::{assembly, eventually, shared, slow_assembly, FakeSettings, MockFactory};
use crate::api::{MediaError, MediaItem, MediaState, MediaStats, PlayerSlotFactory, SlotCall};
use crate::fetch::mock::MockFetcher;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

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
