use super::*;
use super::layer_harness::*;
use super::ingress::{IngressLimits, Pause, PlaceInput};
use crate::api::test_support::{eventually, shared, slow_assembly, FakeSettings, MockFactory, MockSlot};
use crate::api::{MediaError, MediaItem, PlayerSlotFactory, SlotCall};
use async_trait::async_trait;
use std::sync::Arc;

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

#[test]
fn inv_failures_07_closing_the_layer_ends_every_failure_subscription_once_and_a_late_one_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), None)).unwrap();
    let early: Arc<Reported> = shared();
    let _early = layer.subscribe_failures(early.clone());
    let caller = caller();
    caller.block_on(layer.stats()).unwrap();
    assert_eq!(early.ended(), 0, "the failure subscription ended while the layer was open");

    caller.block_on(layer.await_closed());

    // The end goes through the mailbox of the listener, so it lands after the close on its own thread.
    caller.block_on(eventually(|| early.ended() == 1));
    assert_eq!(early.ended(), 1, "closing the layer did not end the failure subscription");
    let late: Arc<Reported> = shared();
    let _late = layer.subscribe_failures(late.clone());
    assert_eq!(late.ended(), 1, "a subscription made after the close stayed open");
    layer.shutdown();
    assert_eq!((early.ended(), late.ended()), (1, 1), "a second close ended a subscription again");
    assert!(early.seen().is_empty() && late.seen().is_empty(), "{:?} {:?}", early.seen(), late.seen());
}
