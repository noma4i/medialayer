use super::*;
use crate::api::test_support::{eventually, MockFactory};
use crate::api::{MediaError, MediaItem, MediaState, PlayerSlotFactory, SlotCall};
use crate::ffi::MediaLayer;
use std::sync::Arc;
use super::ingress_harness::*;

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
    type Case = (&'static str, bool, Option<bool>, Option<u64>, bool, bool);
    let cases: Vec<Case> = vec![
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
            crate::test_signal::fire();
        }
    }
    let _owner = crate::trace::sink_test_guard();
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/dropped-play.m4v";
    let lines = Arc::new(Lines(Default::default()));
    crate::trace::set_trace_sink(Some(lines.clone()));
    layer.player_events().background();
    caller.block_on(eventually(|| layer.queue().audit().entries == 0));
    let place = layer.open_place(MediaItem::bare_video(url)).unwrap();
    place.update(PlaceInput { visible: true, playing: true, ..PlaceInput::default() }).unwrap();

    caller.block_on(eventually(|| lines.0.lock().unwrap().iter().any(|line| line.contains("play dropped: ") && line.contains("dropped-play.m4v") && line.contains("the app is in the background"))));
    crate::trace::set_trace_sink(None);
}

#[test]
fn inv_trace_04_a_suspension_leaves_a_suspend_line_with_its_id_and_groups_and_a_resume_line_with_the_same_id() {
    struct Lines(std::sync::Mutex<Vec<String>>);
    impl crate::trace::TraceSink for Lines {
        fn line(&self, text: String) {
            self.0.lock().unwrap().push(text);
            crate::test_signal::fire();
        }
    }
    let _owner = crate::trace::sink_test_guard();
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::build(config(dir.path(), None, None), Pause::new()).unwrap();
    let caller = caller();
    let lines = Arc::new(Lines(Default::default()));
    crate::trace::set_trace_sink(Some(lines.clone()));

    let suspension = layer.suspend_playback(vec!["chat".to_string(), "voice".to_string()]).unwrap();
    let own_suspend = || lines.0.lock().unwrap().iter().find(|line| line.contains("stage suspend ") && line.contains("allow=[\"chat\", \"voice\"]")).cloned();
    caller.block_on(eventually(|| own_suspend().is_some()));
    let suspend = own_suspend().unwrap();
    let id = suspend.split("stage suspend ").nth(1).unwrap().split(' ').next().unwrap().to_string();
    let exact = format!("stage resume {id}");
    suspension.release();
    caller.block_on(eventually(|| lines.0.lock().unwrap().iter().any(|line| line.ends_with(&exact))));
    let resume = lines.0.lock().unwrap().iter().find(|line| line.ends_with(&exact)).cloned().unwrap();
    crate::trace::set_trace_sink(None);

    assert!(suspend.contains("allow=[\"chat\", \"voice\"]"), "{suspend}");
    assert!(resume.ends_with(&format!("stage resume {id}")), "{resume}");
}

#[derive(Default)]
struct ReadyLog(std::sync::Mutex<Vec<bool>>);

impl crate::ffi::layer::WindowReadyListener for ReadyLog {
    fn changed(&self, ready: bool) {
        self.0.lock().unwrap().push(ready);
    }
}

#[test]
fn inv_window_02_a_failure_that_lands_before_the_followed_page_first_runs_still_makes_the_window_ready() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::open(config(dir.path(), None, None)).unwrap();
    let caller = caller();
    let assembled = caller.block_on(layer.controlled(|assembled| async move { assembled })).unwrap();
    let url = "https://o.test/x.jpg";
    let log = Arc::new(ReadyLog::default());
    let mut readiness = readiness::PageReadiness::new();
    readiness.listen(log.clone());

    readiness.follow(&assembled, caller.handle(), Some(vec![MediaItem::bare(url, crate::api::Kind::Image)]));
    assembled.stage.announce_image(url, Err("offline".to_string()));

    caller.block_on(eventually(|| log.0.lock().unwrap().last() == Some(&true)));
}

#[test]
fn inv_ingress_54_two_one_shot_places_of_a_windowed_note_end_once_across_a_handover() {
    struct Seen(std::sync::Mutex<Vec<MediaState>>);
    impl crate::ffi::StateListener for Seen {
        fn changed(&self, state: MediaState) { self.0.lock().unwrap().push(state); }
        fn failed(&self, _: MediaError) {}
    }
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/note.mp4";
    let windowed = MediaItem::bare_video(url);
    let once = MediaItem { looping: false, ..MediaItem::bare_video(url) };
    let seen = Arc::new(Seen(Default::default()));
    let _subscription = layer.subscribe_state(url.to_string(), seen.clone()).unwrap();

    let window = layer.open_video_window().unwrap();
    window.replace(0, vec![Some(windowed.clone())]).unwrap();
    let row = layer.open_place(once.clone()).unwrap();
    row.update(PlaceInput { visible: true, playing: true, view_token: Some(1), ..PlaceInput::default() }).unwrap();
    let floating = layer.open_place(once.clone()).unwrap();
    floating.update(PlaceInput { visible: true, playing: false, view_token: Some(2), ..PlaceInput::default() }).unwrap();
    row.update(PlaceInput { visible: false, playing: false, view_token: Some(1), ..PlaceInput::default() }).unwrap();
    floating.update(PlaceInput { visible: true, playing: true, view_token: Some(2), ..PlaceInput::default() }).unwrap();
    window.replace(0, vec![Some(windowed.clone())]).unwrap();
    row.update(PlaceInput { visible: true, playing: true, view_token: Some(1), ..PlaceInput::default() }).unwrap();
    floating.update(PlaceInput { visible: true, playing: false, view_token: Some(2), ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))));
    caller.block_on(eventually(|| layer.queue().audit().entries == 0));
    let (index, slot) = factory.slot_holding(url).unwrap();
    let effective = slot.calls().iter().fold(None, |looping, call| match call { SlotCall::Load { key, route, .. } if key == url => Some(route.looping), _ => looping.map(|current| call_looping(call).unwrap_or(current)) });
    layer.player_events().ready(index, slot.generation());
    layer.player_events().visual_ready(index, slot.generation());
    layer.player_events().ended(index, slot.generation());

    assert_eq!(effective, Some(false), "the slot of a one-shot note plays one-shot before its end: {:?}", slot.calls());
    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Ended)));
    assert_eq!(seen.0.lock().unwrap().iter().filter(|state| **state == MediaState::Ended).count(), 1);
}

fn call_looping(call: &SlotCall) -> Option<bool> {
    match format!("{call:?}") {
        text if text.starts_with("Looping { looping: true") => Some(true),
        text if text.starts_with("Looping { looping: false") => Some(false),
        _ => None,
    }
}
