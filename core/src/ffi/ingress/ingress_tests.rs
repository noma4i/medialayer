use super::*;
use crate::api::test_support::{eventually, shared, FakeSettings, MockFactory};
use crate::api::{MediaError, MediaItem, MediaState, PlayerSlotFactory, SlotCall, TonePixels, TonePreparation};
use crate::ffi::{default_limits, default_pool_settings, DeviceMemory, FetchRequest, FetchResponse, Fetcher, MediaLayer, OpenConfig};
use std::sync::Arc;

struct Offline;

#[async_trait::async_trait]
impl Fetcher for Offline {
    async fn fetch(&self, _request: FetchRequest) -> Result<FetchResponse, MediaError> {
        Err(MediaError::Fetch { detail: "offline".to_string() })
    }
}

fn config(dir: &std::path::Path, slots: Option<Arc<dyn PlayerSlotFactory>>, limits: Option<IngressLimits>) -> OpenConfig {
    OpenConfig {
        available_bytes: u64::MAX,
        dir: dir.to_string_lossy().to_string(),
        fetcher: Arc::new(Offline),
        settings: shared::<FakeSettings>(),
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
        ingress: limits,
    }
}

fn caller() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

fn tight(entries: u32, owners: u32, bytes: u64) -> Option<IngressLimits> {
    Some(IngressLimits { entries, owners, bytes })
}

fn update(owner: u64) -> Declared {
    Declared::PlaceUpdate { owner, input: PlaceInput { playing: true, ..PlaceInput::default() } }
}

fn opened(queue: &Arc<InputQueue>, owner: u64) {
    queue.open(owner, OwnerKind::Place, Declared::PlaceOpen { owner, item: MediaItem::bare_video("https://o.test/a.mp4") }).unwrap();
}

fn queued(queue: &Arc<InputQueue>) -> Vec<&'static str> {
    let mut order = Vec::new();
    loop {
        match queue.next() {
            queue::Upcoming::Apply(popped) => order.push(match popped.command {
                Declared::PlaceOpen { .. } => "open",
                Declared::PlaceUpdate { .. } => "update",
                Declared::PlaceSeek { .. } => "seek",
                Declared::KeySeek { .. } => "key-seek",
                Declared::Query { .. } => "query",
                _ => "other",
            }),
            queue::Upcoming::Release(..) => order.push("release"),
            _ => return order,
        }
    }
}

fn freed(queue: &Arc<InputQueue>) {
    let garbage = queue.take_garbage();
    let accounts: Vec<(u64, bool)> = garbage.iter().map(queue::Garbage::account).collect();
    drop(garbage);
    queue.freed(&accounts);
}

#[test]
fn inv_ingress_01_updates_of_one_owner_fold_until_a_query_closes_the_snapshot() {
    let queue = InputQueue::new(IngressLimits::default());
    opened(&queue, 1);
    queue.submit(update(1)).unwrap();
    queue.submit(update(1)).unwrap();
    let (_held, _answer) = queue.query(Query::Pool).unwrap();
    queue.submit(update(1)).unwrap();
    queue.submit(update(1)).unwrap();
    queue.submit(Declared::PlaceSeek { owner: 1, fraction: 0.5 }).unwrap();
    queue.submit(update(1)).unwrap();
    queue.submit(update(1)).unwrap();

    assert_eq!(queued(&queue), vec!["open", "update", "query", "update", "seek", "update"]);
}

#[test]
fn inv_ingress_02_events_are_admitted_up_to_the_entry_limit_and_the_rest_is_refused_not_dropped() {
    let queue = InputQueue::new(IngressLimits { entries: 256, owners: 4, bytes: u64::MAX });
    opened(&queue, 1);
    let refused = (0..300).filter(|_| queue.submit(Declared::PlaceSeek { owner: 1, fraction: 0.5 }).is_err()).count();

    assert_eq!(refused, 300 - 255);
    assert_eq!(queued(&queue).iter().filter(|name| **name == "seek").count(), 255);
}

#[test]
fn inv_ingress_03_a_window_is_weighed_by_backing_capacity_including_none_slots_and_empty_pages() {
    let queue = InputQueue::new(IngressLimits { entries: 64, owners: 4, bytes: 64 * 1024 });
    queue.open(1, OwnerKind::Window, Declared::WindowOpen { owner: 1, kind: WindowKind::Video }).unwrap();
    let blanks: Vec<Option<MediaItem>> = vec![None; 100_000];
    let empty_pages: Vec<Vec<MediaItem>> = vec![Vec::new(); 100_000];

    let video = queue.submit(Declared::VideoWindow { owner: 1, active: 0, sources: Some(Arc::new(blanks)) });
    let images = queue.submit(Declared::ImageWindow { owner: 1, active: 0, pages: Some(Arc::new(empty_pages)) });

    assert!(matches!(video, Err(MediaError::Overloaded { .. })), "{video:?}");
    assert!(matches!(images, Err(MediaError::Overloaded { .. })), "{images:?}");
}

#[test]
fn inv_ingress_04_a_failure_detail_and_a_header_heavy_item_are_weighed() {
    let queue = InputQueue::new(IngressLimits { entries: 64, owners: 4, bytes: 4_096 });
    let detail = Declared::Player(PlayerEvent::Failed { slot: 0, generation: 0, status: 1, detail: "x".repeat(10_000) });
    let item = MediaItem { headers: (0..200).map(|index| (format!("h{index}"), "v".repeat(64))).collect(), ..MediaItem::bare_video("https://o.test/a.mp4") };

    assert!(matches!(queue.submit(detail), Err(MediaError::Overloaded { .. })));
    assert!(matches!(queue.open(1, OwnerKind::Place, Declared::PlaceOpen { owner: 1, item }), Err(MediaError::Overloaded { .. })));
    assert_eq!(queue.audit().bytes, 0);
}

#[test]
fn inv_ingress_05_owners_are_bounded() {
    let queue = InputQueue::new(IngressLimits { entries: 64, owners: 2, bytes: u64::MAX });
    opened(&queue, 1);
    opened(&queue, 2);

    let third = queue.open(3, OwnerKind::Place, Declared::PlaceOpen { owner: 3, item: MediaItem::bare_video("https://o.test/c.mp4") });

    assert!(matches!(third, Err(MediaError::Overloaded { .. })));
}

#[test]
fn inv_ingress_06_an_owner_closes_while_the_queue_is_full_and_frees_its_entries_only_after_the_executor_dropped_them() {
    let queue = InputQueue::new(IngressLimits { entries: 4, owners: 4, bytes: u64::MAX });
    opened(&queue, 1);
    for _ in 0..3 {
        queue.submit(Declared::PlaceSeek { owner: 1, fraction: 0.1 }).unwrap();
    }
    assert!(queue.submit(Declared::PlaceSeek { owner: 1, fraction: 0.1 }).is_err());

    queue.release(1);
    queue.release(1);

    let audit = queue.audit();
    assert_eq!((audit.entries, audit.by_owner, audit.folds, audit.garbage), (3, 0, 0, 1));
    assert_eq!(audit.counted, 4, "the dropped command still holds its place until the executor frees it");
    freed(&queue);
    assert_eq!(queue.audit().counted, 3);
    assert!(matches!(queue.submit(update(1)), Err(MediaError::Closed)));
    assert_eq!(queued(&queue), vec!["seek", "seek", "seek", "release"]);
    assert_eq!(queue.audit().counted, 0);
}

#[test]
fn inv_ingress_07_a_cancelled_query_keeps_its_place_and_parameters_until_the_executor_freed_them() {
    let queue = InputQueue::new(IngressLimits { entries: 2, owners: 2, bytes: u64::MAX });
    let (first, _answer) = queue.query(Query::PlayUrl(MediaItem::bare_video("https://o.test/a.mp4"))).unwrap();
    let (second, _other) = queue.query(Query::Pool).unwrap();
    assert!(queue.query(Query::Pool).is_err());

    drop(first);
    assert!(queue.query(Query::Pool).is_err(), "the cancel freed the place before the payload was dropped");
    freed(&queue);
    let taken = queue.next();
    assert!(matches!(taken, queue::Upcoming::Apply(_)));
    drop(taken);
    drop(second);
    freed(&queue);

    assert_eq!(queue.audit(), queue::Audit::default());
}

#[test]
fn inv_ingress_20_repeated_query_and_cancel_cannot_pass_the_limit_while_the_executor_is_held() {
    let queue = InputQueue::new(IngressLimits { entries: 3, owners: 2, bytes: u64::MAX });
    let item = MediaItem::bare_video("https://o.test/a.mp4");

    let admitted = (0..50).filter(|_| queue.query(Query::PlayUrl(item.clone())).map(drop).is_ok()).count();

    assert_eq!(admitted, 3);
    freed(&queue);
    assert_eq!(queue.audit().counted, 0);
}

#[test]
fn inv_ingress_08_repeated_create_update_release_cycles_leave_no_index_behind_with_the_executor_held() {
    let queue = InputQueue::new(IngressLimits { entries: 64, owners: 8, bytes: u64::MAX });
    for round in 0..200u64 {
        let owner = round % 4 + 1;
        opened(&queue, owner);
        queue.submit(update(owner)).unwrap();
        queue.submit(update(owner)).unwrap();
        queue.submit(Declared::PlaceSeek { owner, fraction: 0.5 }).unwrap();
        queue.release(owner);
        assert_eq!(queued(&queue), vec!["seek", "release"]);
        queue.finish_release(owner);
        freed(&queue);
    }

    assert_eq!(queue.audit(), queue::Audit::default());
}

#[test]
fn inv_ingress_09_an_ended_queue_refuses_with_the_reason_and_answers_waiting_work() {
    let queue = InputQueue::new(IngressLimits::default());
    let (_held, answer) = queue.query(Query::Pool).unwrap();

    queue.end(MediaError::Closed);
    let pending = queue.drain();

    assert!(matches!(queue.submit(update(1)), Err(MediaError::Closed)));
    assert!(matches!(queue.query(Query::Pool), Err(MediaError::Closed)));
    assert_eq!(pending.len(), 1);
    drop(answer);
}

#[test]
fn inv_ingress_10_a_held_executor_accepts_declarations_without_running_them_on_the_caller() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let pause = Pause::new();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), pause.clone()).unwrap();
    let caller = caller();
    let url = "https://o.test/v1.mp4";
    caller.block_on(layer.limits()).unwrap();
    pause.set(true);

    let window = layer.open_video_window().unwrap();
    window.replace(0, vec![Some(MediaItem::bare_video(url))]).unwrap();

    assert!(factory.slot_holding(url).is_none(), "the declaration ran on the caller");
    pause.set(false);
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
}

#[test]
fn inv_ingress_11_two_places_of_one_key_pause_only_when_the_last_playing_owner_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/v1.mp4";
    let _ring = layer.open_video_window().unwrap();
    _ring.replace(0, vec![Some(MediaItem::bare_video(url))]).unwrap();
    let first = layer.open_place(MediaItem::bare_video(url)).unwrap();
    let second = layer.open_place(MediaItem::bare_video(url)).unwrap();
    first.update(PlaceInput { playing: true, ..PlaceInput::default() }).unwrap();
    second.update(PlaceInput { playing: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))));
    let (_, slot) = factory.slot_holding(url).unwrap();

    first.release();
    first.release();
    second.update(PlaceInput { playing: true, rate: Some(2.0), ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Rate { rate: 2.0 })));
    assert!(!slot.calls().contains(&SlotCall::Pause), "the late release of the first place paused the key");

    second.release();
    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));
}

#[test]
fn inv_ingress_12_the_last_active_video_window_wins_and_the_previous_one_returns_when_it_closes() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let (a, b) = ("https://o.test/a.mp4", "https://o.test/b.mp4");
    let first = layer.open_video_window().unwrap();
    let second = layer.open_video_window().unwrap();
    first.replace(0, vec![Some(MediaItem::bare_video(a))]).unwrap();
    let viewer = layer.open_place(MediaItem::bare_video(a)).unwrap();
    viewer.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(a).is_some()));

    second.replace(0, vec![Some(MediaItem::bare_video(b))]).unwrap();
    let other = layer.open_place(MediaItem::bare_video(b)).unwrap();
    other.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(b).is_some()));

    second.release();
    caller.block_on(eventually(|| factory.slot_holding(a).is_some() && factory.slot_holding(b).is_none()));
}

#[test]
fn inv_ingress_13_a_place_with_another_item_of_the_same_url_is_a_new_owner_and_a_late_release_of_the_old_one_is_inert() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/v1.mp4";
    let _ring = layer.open_video_window().unwrap();
    _ring.replace(0, vec![Some(MediaItem::bare_video(url))]).unwrap();
    let old = layer.open_place(MediaItem::bare_video(url)).unwrap();
    old.update(PlaceInput { playing: true, ..PlaceInput::default() }).unwrap();
    let renewed = layer.open_place(MediaItem { looping: false, ..MediaItem::bare_video(url) }).unwrap();
    renewed.update(PlaceInput { playing: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))));
    let (_, slot) = factory.slot_holding(url).unwrap();

    old.release();
    renewed.item_state(MediaState::Loading).unwrap();
    renewed.update(PlaceInput { playing: true, muted: Some(true), ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Mute)));

    assert!(!slot.calls().contains(&SlotCall::Pause));
}

#[test]
fn inv_ingress_14_closing_the_layer_answers_a_query_that_waits_on_held_io() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::build(config(dir.path(), None, None), Pause::new()).unwrap();
    let caller = caller();
    caller.block_on(layer.limits()).unwrap();

    let outcome = caller.block_on(async {
        let waiting = layer.controlled(|_| async { std::future::pending::<()>().await });
        let closing = async {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            layer.shutdown();
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async { tokio::join!(waiting, closing).0 }).await
    });

    let outcome = outcome.expect("the close did not answer the waiting call");
    assert!(matches!(outcome, Err(MediaError::Closed)), "{outcome:?}");
    assert_eq!(layer.queue().audit().counted, 0);
}

#[test]
fn inv_ingress_15_a_native_event_flood_closes_the_layer_with_overloaded_for_the_observers() {
    struct Failed(std::sync::Mutex<Vec<MediaError>>);
    impl crate::ffi::StateListener for Failed {
        fn changed(&self, _: MediaState) {}
        fn failed(&self, error: MediaError) {
            self.0.lock().unwrap().push(error);
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let pause = Pause::new();
    let layer = MediaLayer::build(config(dir.path(), None, tight(8, 8, u64::MAX)), pause.clone()).unwrap();
    let caller = caller();
    caller.block_on(layer.limits()).unwrap();
    let observer = Arc::new(Failed(Default::default()));
    let _subscription = layer.subscribe_state("https://o.test/a.mp4".to_string(), observer.clone()).unwrap();
    caller.block_on(eventually(|| layer.queue().audit().counted == 0));
    pause.set(true);
    let events = layer.player_events();

    for generation in 0..32 {
        events.ready(0, generation);
    }
    pause.set(false);

    caller.block_on(eventually(|| observer.0.lock().unwrap().iter().any(|error| matches!(error, MediaError::Overloaded { .. }))));
    assert_eq!(observer.0.lock().unwrap().len(), 1, "one terminal event per subscription");
}

#[test]
fn inv_ingress_16_a_cancelled_subscription_gets_no_terminal_event() {
    struct Failed(std::sync::Mutex<usize>);
    impl crate::ffi::StateListener for Failed {
        fn changed(&self, _: MediaState) {}
        fn failed(&self, _: MediaError) {
            *self.0.lock().unwrap() += 1;
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let pause = Pause::new();
    let layer = MediaLayer::build(config(dir.path(), None, None), pause.clone()).unwrap();
    let caller = caller();
    caller.block_on(layer.limits()).unwrap();
    let observer = Arc::new(Failed(Default::default()));
    let subscription = layer.subscribe_state("https://o.test/a.mp4".to_string(), observer.clone()).unwrap();
    caller.block_on(eventually(|| layer.queue().audit().counted == 0));
    pause.set(true);
    drop(subscription);

    layer.shutdown();
    pause.set(false);
    caller.block_on(layer.await_closed());

    assert_eq!(*observer.0.lock().unwrap(), 0);
}

#[test]
fn inv_ingress_17_controls_do_not_wait_for_a_held_media_executor() {
    let dir = tempfile::tempdir().unwrap();
    let pause = Pause::new();
    let layer = MediaLayer::build(config(dir.path(), None, None), pause.clone()).unwrap();
    let caller = caller();
    caller.block_on(layer.limits()).unwrap();
    pause.set(true);
    let _place = layer.open_place(MediaItem::bare_video("https://o.test/a.mp4")).unwrap();

    let limits = caller.block_on(async { tokio::time::timeout(std::time::Duration::from_secs(2), layer.limits()).await });

    assert!(matches!(limits, Ok(Ok(_))), "{limits:?}");
}

#[test]
fn inv_ingress_18_the_reservation_of_a_window_lives_with_the_window_after_dequeue() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::build(config(dir.path(), None, None), Pause::new()).unwrap();
    let caller = caller();
    caller.block_on(layer.limits()).unwrap();
    let window = layer.open_video_window().unwrap();
    caller.block_on(eventually(|| layer.queue().audit().entries == 0));
    let before = layer.queue().audit().bytes;
    let sources: Vec<Option<MediaItem>> = (0..50).map(|index| Some(MediaItem::bare_video(&format!("https://o.test/{index}.mp4")))).collect();

    window.replace(0, sources).unwrap();
    caller.block_on(layer.stats()).unwrap();
    caller.block_on(eventually(|| layer.queue().audit().entries == 0));
    let held = layer.queue().audit().bytes;
    window.release();
    caller.block_on(eventually(|| layer.queue().audit() == queue::Audit::default()));

    assert!(held > before, "the applied window left no reservation");
}

struct Admitted(tokio::sync::watch::Sender<usize>, Arc<std::sync::atomic::AtomicBool>);

struct DropFlag(Arc<std::sync::atomic::AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl TonePreparation for Admitted {
    async fn prepare(&self, _source: MediaItem) -> Option<TonePixels> {
        let _dropped = DropFlag(self.1.clone());
        self.0.send_modify(|entered| *entered += 1);
        std::future::pending::<()>().await;
        None
    }
}

#[test]
fn inv_ingress_19_closing_cancels_a_held_tone_preparation_and_none_is_delivered_after_closed() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::build(config(dir.path(), None, None), Pause::new()).unwrap();
    let caller = caller();
    caller.block_on(layer.limits()).unwrap();
    let (entered, mut watch) = tokio::sync::watch::channel(0usize);
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    layer.set_tone_preparation(Arc::new(Admitted(entered, cancelled.clone()))).unwrap();
    let item = MediaItem::bare("https://o.test/a.jpg", crate::api::Kind::Image);
    caller.block_on(async {
        layer.tone(item.clone(), 0.5).await.unwrap();
        watch.wait_for(|entered| *entered == 1).await.unwrap();
    });

    caller.block_on(layer.await_closed());

    assert_eq!(*watch.borrow(), 1);
    assert!(cancelled.load(std::sync::atomic::Ordering::SeqCst), "the held preparation was still alive after the close returned");
    assert!(!layer.owns_runtime());
}

#[test]
fn inv_ingress_21_a_full_shutdown_writes_the_accounting_off_and_releases_the_queue() {
    let dir = tempfile::tempdir().unwrap();
    let pause = Pause::new();
    let layer = MediaLayer::build(config(dir.path(), None, None), pause.clone()).unwrap();
    let caller = caller();
    caller.block_on(layer.limits()).unwrap();
    let queue = Arc::downgrade(layer.queue());
    pause.set(true);
    let place = layer.open_place(MediaItem::bare_video("https://o.test/a.mp4")).unwrap();
    place.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();
    let window = layer.open_video_window().unwrap();
    window.replace(0, vec![Some(MediaItem::bare_video("https://o.test/a.mp4"))]).unwrap();
    let audit_queue = layer.queue().clone();

    layer.shutdown();
    pause.set(false);
    caller.block_on(layer.await_closed());

    assert_eq!(audit_queue.audit(), queue::Audit::default());
    drop((audit_queue, place, window, layer));
    caller.block_on(eventually(|| queue.upgrade().is_none()));
}

#[test]
fn inv_ingress_27_a_changed_source_of_a_shown_key_follows_without_a_second_show() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/v1.mp4";
    let _ring = layer.open_video_window().unwrap();
    _ring.replace(0, vec![Some(MediaItem::bare_video(url))]).unwrap();
    let first = layer.open_place(MediaItem::bare_video(url)).unwrap();
    first.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();
    let second = layer.open_place(MediaItem { cover: Some("https://o.test/cover.jpg".to_string()), ..MediaItem::bare_video(url) }).unwrap();
    second.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| layer.queue().audit().entries == 0));
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    caller.block_on(layer.stats()).unwrap();
    let shown = caller.block_on(layer.controlled(move |assembled| async move { assembled.stage.shown_count(url) })).unwrap();
    assert_eq!(shown, 1, "a changed source of a shown key was shown a second time");
    _ring.release();

    first.release();
    caller.block_on(eventually(|| layer.queue().audit().entries == 0));
    assert!(factory.slot_holding(url).is_some(), "the second place still shows the key");
    second.release();

    caller.block_on(eventually(|| factory.slot_holding(url).is_none()));
}

#[test]
fn inv_ingress_28_a_late_state_of_a_closed_place_never_reaches_a_new_place_of_the_same_url() {
    struct Seen(std::sync::Mutex<Vec<MediaState>>);
    impl crate::ffi::StateListener for Seen {
        fn changed(&self, state: MediaState) {
            self.0.lock().unwrap().push(state);
        }
        fn failed(&self, _: MediaError) {}
    }
    let dir = tempfile::tempdir().unwrap();
    let pause = Pause::new();
    let layer = MediaLayer::build(config(dir.path(), None, None), pause.clone()).unwrap();
    let caller = caller();
    caller.block_on(layer.limits()).unwrap();
    let url = "https://o.test/a.jpg";
    let seen = Arc::new(Seen(Default::default()));
    let _subscription = layer.subscribe_state(url.to_string(), seen.clone()).unwrap();
    caller.block_on(eventually(|| layer.queue().audit().entries == 0));
    pause.set(true);
    let old = layer.open_place(MediaItem::bare(url, crate::api::Kind::Image)).unwrap();
    let renewed = layer.open_place(MediaItem { headers: [("x".to_string(), "1".to_string())].into(), ..MediaItem::bare(url, crate::api::Kind::Image) }).unwrap();
    old.item_state(MediaState::Failed { detail: "late".to_string() }).unwrap();
    old.release();

    assert!(old.item_state(MediaState::Ready).is_err());
    pause.set(false);
    renewed.item_state(MediaState::Loading).unwrap();
    caller.block_on(eventually(|| seen.0.lock().unwrap().contains(&MediaState::Loading)));

    assert!(!seen.0.lock().unwrap().iter().any(|state| matches!(state, MediaState::Failed { .. })));
}

#[test]
fn inv_ingress_29_a_suspension_keeps_its_groups_reserved_until_it_is_released() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::build(config(dir.path(), None, None), Pause::new()).unwrap();
    let caller = caller();
    caller.block_on(layer.limits()).unwrap();
    let before = layer.queue().audit().bytes;

    let suspension = layer.suspend_playback(vec!["chat".to_string(), "voice".to_string()]).unwrap();
    caller.block_on(eventually(|| layer.queue().audit().entries == 0));
    let held = layer.queue().audit().bytes;
    suspension.release();
    caller.block_on(eventually(|| layer.queue().audit().owners == 0));

    assert!(held > before, "the applied suspension left no reservation");
    assert_eq!(layer.queue().audit().bytes, before);
}

#[test]
fn inv_ingress_30_a_move_costs_the_same_visits_for_any_list_length() {
    let mut visits = Vec::new();
    for length in [20usize, 200, 2000] {
        let queue = InputQueue::new(IngressLimits { entries: 64, owners: 4, bytes: u64::MAX });
        queue.open(1, OwnerKind::Window, Declared::WindowOpen { owner: 1, kind: WindowKind::Video }).unwrap();
        let sources: Vec<Option<MediaItem>> = (0..length).map(|index| Some(MediaItem::bare_video(&format!("https://o.test/{index}.mp4")))).collect();
        queue.submit(Declared::VideoWindow { owner: 1, active: 0, sources: Some(Arc::new(sources)) }).unwrap();
        let before = weight::VISITS.with(|visits| visits.get());

        queue.submit(Declared::VideoWindow { owner: 1, active: 5, sources: None }).unwrap();

        visits.push(weight::VISITS.with(|visits| visits.get()) - before);
    }
    assert_eq!(visits, vec![0, 0, 0]);
}

#[test]
fn inv_ingress_31_closing_with_folded_garbage_writes_off_exactly_to_zero() {
    let queue = InputQueue::new(IngressLimits { entries: 64, owners: 4, bytes: u64::MAX });
    queue.open(1, OwnerKind::Window, Declared::WindowOpen { owner: 1, kind: WindowKind::Video }).unwrap();
    for round in 0..5 {
        let sources: Vec<Option<MediaItem>> = (0..10).map(|index| Some(MediaItem::bare_video(&format!("https://o.test/{round}-{index}.mp4")))).collect();
        queue.submit(Declared::VideoWindow { owner: 1, active: 0, sources: Some(Arc::new(sources)) }).unwrap();
    }
    queue.release(1);
    assert!(queue.audit().garbage > 0);

    let drained = queue.drain();

    assert!(!drained.is_empty());
    assert_eq!(queue.audit(), queue::Audit::default());
    assert!(queue.end_error().is_none(), "the write-off found an accounting mismatch");
}

#[test]
fn inv_ingress_32_a_failed_state_detail_is_weighed() {
    let queue = InputQueue::new(IngressLimits { entries: 64, owners: 4, bytes: 4_096 });
    queue.open(1, OwnerKind::Place, Declared::PlaceOpen { owner: 1, item: MediaItem::bare_video("https://o.test/a.mp4") }).unwrap();

    let verdict = queue.submit(Declared::PlaceState { owner: 1, state: MediaState::Failed { detail: "x".repeat(10_000) } });

    assert!(matches!(verdict, Err(MediaError::Overloaded { .. })));
}

struct DropProbe {
    queue: Arc<InputQueue>,
    unlocked: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl TonePreparation for DropProbe {
    async fn prepare(&self, _source: MediaItem) -> Option<TonePixels> {
        None
    }
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.unlocked.store(self.queue.is_unlocked(), std::sync::atomic::Ordering::SeqCst);
    }
}

fn probe(queue: &Arc<InputQueue>) -> (Declared, Arc<std::sync::atomic::AtomicBool>) {
    let unlocked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    (Declared::Preparation(Arc::new(DropProbe { queue: queue.clone(), unlocked: unlocked.clone() })), unlocked)
}

#[test]
fn inv_ingress_33_a_refused_payload_is_dropped_after_the_lock_is_released() {
    let queue = InputQueue::new(IngressLimits { entries: 1, owners: 2, bytes: u64::MAX });
    opened(&queue, 1);
    let (rejected, dropped_unlocked) = probe(&queue);

    let verdict = queue.submit(rejected);

    assert!(matches!(verdict, Err(MediaError::Overloaded { .. })));
    assert!(dropped_unlocked.load(std::sync::atomic::Ordering::SeqCst), "the refused payload was dropped under the lock");
    let tight = InputQueue::new(IngressLimits { entries: 8, owners: 2, bytes: 8 });
    let (by_bytes, bytes_unlocked) = probe(&tight);
    let verdict = tight.submit(by_bytes);
    assert!(matches!(verdict, Err(MediaError::Overloaded { .. })));
    assert!(bytes_unlocked.load(std::sync::atomic::Ordering::SeqCst), "the payload refused by bytes was dropped under the lock");
}

#[test]
fn inv_ingress_34_a_release_waits_behind_the_entries_accepted_before_it() {
    let queue = InputQueue::new(IngressLimits::default());
    opened(&queue, 1);
    queue.submit(update(1)).unwrap();
    queue.open(2, OwnerKind::Place, Declared::PlaceOpen { owner: 2, item: MediaItem::bare_video("https://o.test/a.mp4") }).unwrap();
    queue.submit(update(2)).unwrap();

    queue.release(1);

    assert_eq!(queued(&queue), vec!["open", "update", "release"]);
}

#[test]
fn inv_ingress_35_a_place_opened_before_the_release_of_another_place_of_the_same_url_keeps_the_slot_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let pause = Pause::new();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), pause.clone()).unwrap();
    let caller = caller();
    let url = "https://o.test/v1.mp4";
    let old = layer.open_place(MediaItem::bare_video(url)).unwrap();
    old.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let (_, slot) = factory.slot_holding(url).unwrap();
    pause.set(true);

    let renewed = layer.open_place(MediaItem { headers: [("x".to_string(), "1".to_string())].into(), ..MediaItem::bare_video(url) }).unwrap();
    renewed.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();
    old.release();
    pause.set(false);
    caller.block_on(eventually(|| layer.queue().audit().entries == 0 && layer.queue().audit().releases == 0));

    assert!(!slot.calls().contains(&SlotCall::Unload), "{:?}", slot.calls());
    assert_eq!(factory.disposed_slots(), 0);
    assert!(factory.slot_holding(url).is_some());
}

#[test]
fn inv_ingress_36_without_any_rate_opinion_a_key_returns_to_normal_speed() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/v1.mp4";
    let _ring = layer.open_video_window().unwrap();
    _ring.replace(0, vec![Some(MediaItem::bare_video(url))]).unwrap();
    let fast = layer.open_place(MediaItem::bare_video(url)).unwrap();
    fast.update(PlaceInput { playing: true, rate: Some(2.0), ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Rate { rate: 2.0 }))));
    let (_, slot) = factory.slot_holding(url).unwrap();
    let plain = layer.open_place(MediaItem::bare_video(url)).unwrap();
    plain.update(PlaceInput { playing: true, ..PlaceInput::default() }).unwrap();

    fast.release();

    caller.block_on(eventually(|| slot.calls().iter().rposition(|call| matches!(call, SlotCall::Rate { .. })).is_some_and(|last| slot.calls()[last] == SlotCall::Rate { rate: 1.0 })));
    drop(plain);
}

fn key_seek(key: &str, fraction: f64) -> Declared {
    Declared::KeySeek { key: key.to_string(), fraction }
}

fn pending_fractions(queue: &Arc<InputQueue>) -> Vec<(String, f64)> {
    let mut seen = Vec::new();
    while let queue::Upcoming::Apply(popped) = queue.next() {
        if let Declared::KeySeek { key, fraction } = popped.command {
            seen.push((key, fraction));
        }
    }
    seen
}

#[test]
fn inv_ingress_37_key_seeks_of_one_key_fold_into_the_last_fraction_and_other_keys_stay_apart() {
    let queue = InputQueue::new(IngressLimits::default());
    queue.submit(key_seek("a", 0.1)).unwrap();
    queue.submit(key_seek("b", 0.9)).unwrap();
    queue.submit(key_seek("a", 0.2)).unwrap();
    queue.submit(key_seek("a", 0.3)).unwrap();

    assert_eq!(pending_fractions(&queue), vec![("a".to_string(), 0.3), ("b".to_string(), 0.9)]);
}

fn pending_muted_all(queue: &Arc<InputQueue>) -> Vec<bool> {
    let mut seen = Vec::new();
    while let queue::Upcoming::Apply(popped) = queue.next() {
        if let Declared::MutedAll(muted) = popped.command {
            seen.push(muted);
        }
    }
    seen
}

#[test]
fn inv_ingress_42_mute_all_calls_fold_into_the_last_value_until_they_are_drained() {
    let queue = InputQueue::new(IngressLimits::default());
    queue.submit(Declared::MutedAll(true)).unwrap();
    queue.submit(Declared::MutedAll(false)).unwrap();

    assert_eq!(pending_muted_all(&queue), vec![false]);
}

#[test]
fn inv_ingress_38_a_key_seek_after_a_barrier_never_folds_into_one_before_it_and_keeps_its_order() {
    let queue = InputQueue::new(IngressLimits::default());
    opened(&queue, 1);
    queue.submit(key_seek("a", 0.1)).unwrap();
    queue.submit(update(1)).unwrap();
    let (_held, _answer) = queue.query(Query::Pool).unwrap();
    queue.submit(key_seek("a", 0.2)).unwrap();
    queue.submit(key_seek("a", 0.3)).unwrap();

    assert_eq!(queued(&queue), vec!["open", "key-seek", "update", "query", "key-seek"]);
}

#[test]
fn inv_ingress_39_a_key_seek_is_weighed_and_refused_as_overloaded_at_the_limits() {
    let heavy = InputQueue::new(IngressLimits { entries: 64, owners: 4, bytes: 4_096 });
    let full = InputQueue::new(IngressLimits { entries: 1, owners: 4, bytes: u64::MAX });
    full.submit(key_seek("a", 0.1)).unwrap();

    let long = heavy.submit(key_seek(&"k".repeat(10_000), 0.5));
    let beyond = full.submit(key_seek("b", 0.1));
    let folded = full.submit(key_seek("a", 0.2));

    assert!(matches!(long, Err(MediaError::Overloaded { .. })), "{long:?}");
    assert!(matches!(beyond, Err(MediaError::Overloaded { .. })), "{beyond:?}");
    assert!(folded.is_ok());
    assert_eq!(heavy.audit().bytes, 0);
}

#[test]
fn inv_ingress_40_a_key_seek_of_an_unloaded_key_applies_when_the_key_loads() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/v1.mp4";

    let creating = factory.create_gate.hold();

    layer.seek(url.to_string(), 0.5).unwrap();
    let ring = layer.open_video_window().unwrap();
    ring.replace(0, vec![Some(MediaItem::bare_video(url))]).unwrap();
    let place = layer.open_place(MediaItem::bare_video(url)).unwrap();
    place.update(PlaceInput { playing: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.create_gate.entered() == 1 && layer.queue().audit().counted == 0));
    drop(creating);
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let (index, slot) = factory.slot_holding(url).unwrap();
    layer.player_events().ready(index, slot.generation());

    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Seek { position_ms: 30_000 })));
}

#[test]
fn inv_ingress_41_a_closed_core_refuses_the_key_seek_to_the_caller() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::build(config(dir.path(), None, None), Pause::new()).unwrap();
    let caller = caller();
    caller.block_on(layer.await_closed());

    let refused = layer.seek("https://o.test/v1.mp4".to_string(), 0.5);

    assert!(matches!(refused, Err(MediaError::Closed)), "{refused:?}");
}

#[test]
fn inv_ingress_44_a_note_that_ended_and_was_rewound_by_its_closing_place_returns_at_the_start_when_mounted_again() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/note.m4a";
    let note = MediaItem { looping: false, ..MediaItem::bare(url, crate::api::Kind::Audio) };

    let place = layer.open_place(note.clone()).unwrap();
    place.update(PlaceInput { playing: true, visible: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let (index, slot) = factory.slot_holding(url).unwrap();
    {
        let mut sample = slot.sample.lock().unwrap();
        sample.position_ms = 60_000;
        sample.duration_ms = Some(60_000);
    }
    layer.player_events().ready(index, slot.generation());
    layer.player_events().ended(index, slot.generation());
    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Seek { position_ms: 0 })));

    place.seek(0.0).unwrap();
    place.release();
    caller.block_on(eventually(|| factory.slot_holding(url).is_none()));
    let again = layer.open_place(note).unwrap();
    again.update(PlaceInput { playing: true, visible: true, ..PlaceInput::default() }).unwrap();

    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let (_, slot) = factory.slot_holding(url).unwrap();
    caller.block_on(eventually(|| layer.queue().audit().counted == 0));
    let calls = slot.calls();
    let load = calls.iter().rposition(|call| matches!(call, SlotCall::Load { .. })).unwrap();
    assert!(!calls[load..].iter().any(|call| matches!(call, SlotCall::Seek { position_ms } if *position_ms > 0)), "{calls:?}");
}

#[test]
fn inv_ingress_45_a_seek_submitted_just_before_the_release_of_its_place_is_kept_for_the_next_place_of_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/note.m4a";
    let note = MediaItem { looping: false, ..MediaItem::bare(url, crate::api::Kind::Audio) };
    let place = layer.open_place(note.clone()).unwrap();
    place.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let (index, slot) = factory.slot_holding(url).unwrap();
    slot.sample.lock().unwrap().duration_ms = Some(60_000);
    layer.player_events().ready(index, slot.generation());

    place.seek(0.5).unwrap();
    place.release();
    caller.block_on(eventually(|| factory.slot_holding(url).is_none()));
    let again = layer.open_place(note).unwrap();
    again.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();

    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let (_, slot) = factory.slot_holding(url).unwrap();
    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Seek { position_ms: 30_000 })));
}

#[test]
fn inv_ingress_46_a_clip_that_is_not_looping_ends_its_place_when_a_window_and_a_place_declare_the_same_url() {
    struct Seen(std::sync::Mutex<Vec<MediaState>>);
    impl crate::ffi::StateListener for Seen {
        fn changed(&self, state: MediaState) {
            self.0.lock().unwrap().push(state);
        }
        fn failed(&self, _: MediaError) {}
    }
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/replay.m4v";
    let once = MediaItem { looping: false, ..MediaItem::bare_video(url) };
    let seen = Arc::new(Seen(Default::default()));
    let _subscription = layer.subscribe_state(url.to_string(), seen.clone()).unwrap();

    let window = layer.open_video_window().unwrap();
    window.replace(0, vec![Some(once.clone())]).unwrap();
    let place = layer.open_place(once).unwrap();
    place.update(PlaceInput { visible: true, playing: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))));
    let (index, slot) = factory.slot_holding(url).unwrap();
    layer.player_events().ready(index, slot.generation());
    layer.player_events().visual_ready(index, slot.generation());
    layer.player_events().ended(index, slot.generation());

    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Ended)));
}

#[test]
fn inv_ingress_47_a_slot_a_window_prerolled_gets_play_when_a_place_of_the_same_url_shows_and_plays_later() {
    let cases: Vec<(&str, bool, Option<bool>, Option<u64>, bool, bool)> = vec![
        ("plain", false, None, None, false, false),
        ("ready before", true, None, None, false, false),
        ("muted and view", false, Some(true), Some(1), false, false),
        ("muted and view, ready before", true, Some(true), Some(1), false, false),
        ("split update", false, Some(true), Some(1), true, false),
        ("place loops", false, Some(true), Some(1), true, true),
    ];
    for (name, ready_before, muted, token, split, place_loops) in cases {
        let dir = tempfile::tempdir().unwrap();
        let factory: Arc<MockFactory> = Arc::default();
        let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
        let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
        let caller = caller();
        let url = "https://o.test/replay.m4v";
        let once = MediaItem { looping: false, ..MediaItem::bare_video(url) };

        let window = layer.open_video_window().unwrap();
        window.replace(0, vec![Some(once.clone())]).unwrap();
        caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Preroll))));
        let (index, slot) = factory.slot_holding(url).unwrap();
        if ready_before {
            layer.player_events().ready(index, slot.generation());
        }
        let place = layer.open_place(MediaItem { looping: place_loops, ..once }).unwrap();
        if split {
            place.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();
        }
        place.update(PlaceInput { visible: true, playing: true, muted, view_token: token, ..PlaceInput::default() }).unwrap();

        caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Play)));
        println!("case {name}: play issued");
    }
}

#[test]
fn inv_ingress_48_a_play_that_is_not_issued_leaves_a_trace_line_with_its_reason() {
    struct Lines(std::sync::Mutex<Vec<String>>);
    impl crate::trace::TraceSink for Lines {
        fn line(&self, text: String) {
            self.0.lock().unwrap().push(text);
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/dropped-play.m4v";
    let lines = Arc::new(Lines(Default::default()));
    crate::trace::set_trace_sink(Some(lines.clone()));
    layer.player_events().background();
    let place = layer.open_place(MediaItem::bare_video(url)).unwrap();
    place.update(PlaceInput { visible: true, playing: true, ..PlaceInput::default() }).unwrap();

    caller.block_on(eventually(|| lines.0.lock().unwrap().iter().any(|line| line.contains("play dropped: ") && line.contains("dropped-play.m4v") && line.contains("the app is in the background"))));
    crate::trace::set_trace_sink(None);
}
