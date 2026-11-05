use super::*;
use crate::api::test_support::eventually;
use crate::api::{MediaError, MediaItem, MediaState, TonePixels, TonePreparation};
use crate::ffi::MediaLayer;
use std::sync::Arc;
use super::ingress_harness::*;

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

#[async_trait::async_trait]
impl TonePreparation for Admitted {
    async fn prepare(&self, _source: MediaItem) -> Option<TonePixels> {
        let _dropped = DropFlag(self.1.clone());
        self.0.send_modify(|entered| *entered += 1);
        std::future::pending::<()>().await;
        None
    }
}

struct DropFlag(Arc<std::sync::atomic::AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
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
