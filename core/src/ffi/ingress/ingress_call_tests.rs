use super::*;
use crate::api::test_support::{eventually, MockFactory, MockSlot};
use crate::api::{MediaError, MediaItem, PlayerSlotFactory, SlotCall};
use crate::ffi::MediaLayer;
use std::sync::Arc;
use super::ingress_harness::*;

const URL: &str = "https://o.test/v1.mp4";
const OTHER: &str = "https://o.test/v2.mp4";

struct Rig {
    _dir: tempfile::TempDir,
    layer: Arc<MediaLayer>,
    factory: Arc<MockFactory>,
    caller: tokio::runtime::Runtime,
    _window: Arc<crate::ffi::MediaWindow>,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let factory: Arc<MockFactory> = Arc::default();
        let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
        let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
        let window = layer.open_video_window().unwrap();
        window.replace(0, vec![Some(MediaItem::bare_video(URL))]).unwrap();
        Rig { _dir: dir, layer, factory, caller: caller(), _window: window }
    }

    fn playing(&self) -> PlaceInput {
        PlaceInput { playing: true, ..PlaceInput::default() }
    }

    fn slot(&self) -> Arc<MockSlot> {
        self.caller.block_on(eventually(|| self.factory.slot_holding(URL).is_some()));
        self.factory.slot_holding(URL).unwrap().1
    }

    fn plays(&self, slot: &MockSlot) -> usize {
        slot.calls().iter().filter(|call| matches!(call, SlotCall::Play)).count()
    }

    /// Everything declared before this call is applied when it returns.
    fn settled(&self) {
        let _ = self.caller.block_on(self.layer.play_url(MediaItem::bare_video("https://o.test/barrier.mp4")));
    }

    fn started(&self) -> (Arc<crate::ffi::MediaPlace>, Arc<MockSlot>) {
        let place = self.layer.open_place(MediaItem::bare_video(URL)).unwrap();
        place.update(self.playing()).unwrap();
        let slot = self.slot();
        self.caller.block_on(eventually(|| self.plays(&slot) == 1));
        (place, slot)
    }
}

#[test]
fn inv_call_01_a_hold_pauses_every_playing_key() {
    let rig = Rig::new();
    let (_place, slot) = rig.started();

    let _hold = rig.layer.hold_call().unwrap();

    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));
}

#[test]
fn inv_call_02_a_play_declared_during_a_hold_gives_no_play_and_a_play_declared_after_the_release_plays() {
    let rig = Rig::new();
    let hold = rig.layer.hold_call().unwrap();
    let place = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    place.update(rig.playing()).unwrap();
    let slot = rig.slot();
    rig.settled();
    assert_eq!(rig.plays(&slot), 0, "the stage played during a call");

    hold.release();
    place.update(PlaceInput { visible: true, ..rig.playing() }).unwrap();

    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
}

#[test]
fn inv_call_03_a_release_restores_nothing_for_a_place_that_kept_playing() {
    let rig = Rig::new();
    let (place, slot) = rig.started();
    let hold = rig.layer.hold_call().unwrap();
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));

    hold.release();
    rig.settled();
    rig.settled();

    assert_eq!(rig.plays(&slot), 1, "the release resumed the place");
    place.update(PlaceInput { visible: true, ..rig.playing() }).unwrap();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 2));
}

#[test]
fn inv_call_04_a_suspension_released_during_a_hold_restores_nothing() {
    let rig = Rig::new();
    let suspension = rig.layer.suspend_playback(Vec::new()).unwrap();
    rig.settled();
    let (place, slot) = {
        let place = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
        place.update(rig.playing()).unwrap();
        (place, rig.slot())
    };
    let _hold = rig.layer.hold_call().unwrap();

    suspension.release();
    rig.settled();
    place.update(PlaceInput { muted: Some(true), ..rig.playing() }).unwrap();
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Mute)));

    assert_eq!(rig.plays(&slot), 0, "the suspension release played during a call: {:?}", slot.calls());
}

#[test]
fn inv_call_01_the_first_hold_hands_play_to_no_other_waiting_key() {
    let rig = Rig::new();
    let waiting = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    waiting.update(rig.playing()).unwrap();
    let slot = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
    let newer = rig.layer.open_place(MediaItem::bare_video(OTHER)).unwrap();
    newer.update(PlaceInput { visible: true, ..rig.playing() }).unwrap();
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));

    let _hold = rig.layer.hold_call().unwrap();
    rig.settled();
    waiting.update(PlaceInput { muted: Some(true), ..rig.playing() }).unwrap();
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Mute)));

    assert_eq!(rig.plays(&slot), 1, "the hold handed play to the waiting key: {:?}", slot.calls());
}

#[test]
fn inv_call_03_closing_a_sibling_place_after_the_release_restores_nothing() {
    let rig = Rig::new();
    let (place, slot) = rig.started();
    let sibling = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    sibling.update(rig.playing()).unwrap();
    let hold = rig.layer.hold_call().unwrap();
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));

    hold.release();
    sibling.release();
    place.update(PlaceInput { muted: Some(true), ..PlaceInput::default() }).unwrap();
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Mute)));

    assert_eq!(rig.plays(&slot), 1, "closing a sibling resumed the key: {:?}", slot.calls());
}

#[test]
fn inv_call_14_a_peek_declared_during_a_hold_gives_no_play() {
    let rig = Rig::new();
    let _hold = rig.layer.hold_call().unwrap();
    let place = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    place.update(PlaceInput { peeking: true, ..PlaceInput::default() }).unwrap();
    let slot = rig.slot();
    rig.settled();
    place.update(PlaceInput { peeking: true, view_token: Some(7), ..PlaceInput::default() }).unwrap();
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Attach { view_token: 7 })));

    assert_eq!(rig.plays(&slot), 0, "a peek played during a call: {:?}", slot.calls());
}

#[test]
fn inv_call_14_the_first_hold_pauses_a_running_peek() {
    let rig = Rig::new();
    let place = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    place.update(PlaceInput { peeking: true, ..PlaceInput::default() }).unwrap();
    let slot = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));

    let _hold = rig.layer.hold_call().unwrap();

    rig.caller.block_on(eventually(|| {
        let calls = slot.calls();
        calls.iter().rposition(|call| *call == SlotCall::Play).is_some_and(|play| calls[play..].contains(&SlotCall::Pause))
    }));
}

#[test]
fn inv_call_05_play_url_is_refused_during_a_hold_and_answers_after_the_last_release() {
    let rig = Rig::new();
    let (first, second) = (rig.layer.hold_call().unwrap(), rig.layer.hold_call().unwrap());
    let item = MediaItem::bare_video(URL);

    assert!(matches!(rig.caller.block_on(rig.layer.play_url(item.clone())), Err(MediaError::InCall)));
    first.release();
    first.release();
    assert!(matches!(rig.caller.block_on(rig.layer.play_url(item.clone())), Err(MediaError::InCall)), "one hold left, the call runs");
    second.release();

    assert!(rig.caller.block_on(rig.layer.play_url(item)).is_ok());
}

#[test]
fn inv_call_06_dropping_the_hold_releases_it() {
    let rig = Rig::new();
    let hold = rig.layer.hold_call().unwrap();
    assert!(matches!(rig.caller.block_on(rig.layer.play_url(MediaItem::bare_video(URL))), Err(MediaError::InCall)));

    drop(hold);

    let item = MediaItem::bare_video(URL);
    assert!(rig.caller.block_on(rig.layer.play_url(item)).is_ok());
}

#[test]
fn inv_call_07_a_hold_takes_its_place_in_the_arrival_order() {
    let rig = Rig::new();
    let place = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    place.update(rig.playing()).unwrap();
    let slot = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
    let _hold = rig.layer.hold_call().unwrap();
    place.update(PlaceInput { visible: true, ..rig.playing() }).unwrap();

    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));
    rig.settled();

    assert_eq!(rig.plays(&slot), 1, "an update accepted after the hold played");
}
