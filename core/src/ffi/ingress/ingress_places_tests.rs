use super::*;
use crate::api::test_support::{eventually, MockFactory};
use crate::api::{MediaError, MediaItem, MediaState, PlayerSlotFactory, SlotCall};
use crate::ffi::MediaLayer;
use std::sync::Arc;
use super::ingress_harness::*;

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

#[test]
fn inv_ingress_57_retry_reloads_only_a_failed_key() {
    struct States(std::sync::Mutex<Vec<MediaState>>);
    impl crate::ffi::StateListener for States {
        fn changed(&self, state: MediaState) {
            self.0.lock().unwrap().push(state);
        }
        fn failed(&self, _: MediaError) {}
    }
    struct Attempts(std::sync::Mutex<Vec<u64>>);
    impl crate::ffi::PlaybackListener for Attempts {
        fn changed(&self, state: crate::api::PlaybackState) {
            self.0.lock().unwrap().push(state.attempt);
        }
        fn failed(&self, _: MediaError) {}
    }
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::build(config(dir.path(), None, None), Pause::new()).unwrap();
    let caller = caller();
    caller.block_on(layer.limits()).unwrap();
    let rows = [
        ("https://o.test/ready.jpg", MediaState::Ready, MediaState::Ready, 0),
        ("https://o.test/failed.jpg", MediaState::Failed { detail: "the network dropped".to_string() }, MediaState::Loading, 1),
    ];
    let mut held = Vec::new();
    for (url, reported, _, _) in &rows {
        let states = Arc::new(States(Default::default()));
        let subscription = layer.subscribe_state(url.to_string(), states.clone()).unwrap();
        let place = layer.open_place(MediaItem::bare(url, crate::api::Kind::Image)).unwrap();
        let attempts = Arc::new(Attempts(Default::default()));
        let playback = place.subscribe_playback(attempts.clone()).unwrap();
        place.item_state(MediaState::Loading).unwrap();
        place.item_state(reported.clone()).unwrap();
        caller.block_on(eventually(|| states.0.lock().unwrap().last() == Some(reported)));
        held.push((states, attempts, subscription, playback, place));
    }

    for (url, _, _, _) in &rows {
        caller.block_on(layer.retry(MediaItem::bare(url, crate::api::Kind::Image))).unwrap();
    }
    caller.block_on(layer.stats()).unwrap();

    for ((url, _, state, attempt), (states, attempts, _, _, _)) in rows.iter().zip(&held).rev() {
        caller.block_on(eventually(|| states.0.lock().unwrap().last() == Some(state) && attempts.0.lock().unwrap().last() == Some(attempt)));
        assert_eq!(states.0.lock().unwrap().last(), Some(state), "{url}");
        assert_eq!(attempts.0.lock().unwrap().last(), Some(attempt), "{url}");
    }
}
