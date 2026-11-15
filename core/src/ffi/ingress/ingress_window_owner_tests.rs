use super::*;
use crate::api::test_support::{eventually, MockFactory, MockSlot};
use crate::api::{MediaItem, PlayerSlotFactory, SlotCall};
use crate::ffi::MediaLayer;
use std::sync::Arc;
use super::ingress_harness::*;

const FEED: &str = "https://o.test/feed.m4v";
const OTHER: &str = "https://o.test/other.m4v";

struct Feed {
    factory: Arc<MockFactory>,
    layer: Arc<MediaLayer>,
    caller: tokio::runtime::Runtime,
    slot: Arc<MockSlot>,
    window: Arc<crate::ffi::MediaWindow>,
    _dir: tempfile::TempDir,
}

impl Feed {
    fn open() -> Feed {
        let dir = tempfile::tempdir().unwrap();
        let factory: Arc<MockFactory> = Arc::default();
        let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
        let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
        let caller = caller();
        let window = layer.open_video_window().unwrap();
        window.replace(0, vec![Some(MediaItem::bare_video(FEED))]).unwrap();
        caller.block_on(eventually(|| factory.slot_holding(FEED).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Preroll))));
        let slot = factory.slot_holding(FEED).unwrap().1;
        Feed { factory, layer, caller, slot, window, _dir: dir }
    }

    fn settle(&self) {
        let probe = self.layer.open_place(MediaItem::bare_video(OTHER)).unwrap();
        probe.update(PlaceInput { visible: true, playing: true, ..PlaceInput::default() }).unwrap();
        self.caller.block_on(eventually(|| self.factory.slot_holding(OTHER).is_some()));
        probe.release();
    }

    fn unchanged_after(&self, before: &[SlotCall], step: &str) {
        self.settle();
        assert_eq!(self.slot.calls(), before, "{step}");
    }
}

#[test]
fn inv_ingress_49_a_new_window_without_sources_leaves_the_stage_to_the_window_that_has_them() {
    let feed = Feed::open();
    let before = feed.slot.calls();
    let offscreen = feed.layer.open_video_window().unwrap();

    offscreen.replace(0, Vec::new()).unwrap();
    feed.unchanged_after(&before, "empty replace of a new active window");
    offscreen.move_to(0).unwrap();
    feed.unchanged_after(&before, "move of a window without sources");
    offscreen.set_active(false).unwrap();
    feed.unchanged_after(&before, "deactivation");
    assert!(!before.contains(&SlotCall::Unload));
}

#[test]
fn inv_ingress_50_activating_a_window_without_sources_does_not_take_the_stage() {
    let feed = Feed::open();
    let before = feed.slot.calls();
    let offscreen = feed.layer.open_video_window().unwrap();
    offscreen.set_active(false).unwrap();
    feed.unchanged_after(&before, "deactivation");

    offscreen.set_active(true).unwrap();
    feed.unchanged_after(&before, "activation without sources");
    offscreen.replace(0, vec![None]).unwrap();
    feed.unchanged_after(&before, "a list of empty entries declares nothing");
    assert!(!feed.slot.calls().contains(&SlotCall::Unload));
}

#[test]
fn inv_ingress_51_a_window_that_gets_its_first_sources_while_active_wins_the_stage() {
    let feed = Feed::open();
    let offscreen = feed.layer.open_video_window().unwrap();
    offscreen.replace(0, Vec::new()).unwrap();

    offscreen.replace(0, vec![Some(MediaItem::bare_video(OTHER))]).unwrap();

    feed.caller.block_on(eventually(|| feed.slot.calls().contains(&SlotCall::Unload)));
}

#[test]
fn inv_ingress_52_the_owner_of_the_stage_that_empties_its_sources_clears_the_stage() {
    let feed = Feed::open();
    let offscreen = feed.layer.open_video_window().unwrap();
    offscreen.replace(0, Vec::new()).unwrap();

    feed.window.replace(0, Vec::new()).unwrap();

    feed.caller.block_on(eventually(|| feed.slot.calls().contains(&SlotCall::Unload)));
}

#[test]
fn inv_ingress_53_closing_a_window_that_never_owned_the_stage_leaves_the_stage_alone() {
    let feed = Feed::open();
    let before = feed.slot.calls();
    let offscreen = feed.layer.open_video_window().unwrap();
    offscreen.replace(0, Vec::new()).unwrap();

    offscreen.release();

    feed.unchanged_after(&before, "release of a window without sources");
    assert!(!feed.slot.calls().contains(&SlotCall::Unload));
}
