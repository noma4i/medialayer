use super::*;
use crate::api::test_support::{eventually, MockFactory, MockSlot};
use crate::api::{MediaError, MediaItem, PlayerSlotFactory, SlotCall};
use crate::audio::{AppliedResult, AudioAction, AudioFocus, AudioInput, CallKind, MicrophoneAvailability, RecordingKind, SessionWant};
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
    inputs: Arc<crate::ffi::AudioInputs>,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let factory: Arc<MockFactory> = Arc::default();
        let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
        let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
        let window = layer.open_video_window().unwrap();
        window.replace(0, vec![Some(MediaItem::bare_video(URL))]).unwrap();
        let inputs = layer.audio_inputs();
        Rig { _dir: dir, layer, factory, caller: caller(), _window: window, inputs }
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

    fn audio(&self, input: AudioInput) -> Vec<AudioAction> {
        self.inputs.send(input).unwrap()
    }

    fn external(&self, active: bool) {
        self.audio(AudioInput::PhoneCall { active });
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
fn inv_call_01_a_call_pauses_every_playing_key() {
    let rig = Rig::new();
    let (_place, slot) = rig.started();

    rig.external(true);

    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));
}

#[test]
fn inv_call_02_a_play_declared_during_a_call_gives_no_play_and_a_play_declared_after_its_end_plays() {
    let rig = Rig::new();
    rig.external(true);
    let place = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    place.update(rig.playing()).unwrap();
    let slot = rig.slot();
    rig.settled();
    assert_eq!(rig.plays(&slot), 0, "the stage played during a call");

    rig.external(false);
    place.update(PlaceInput { visible: true, ..rig.playing() }).unwrap();

    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
}

#[test]
fn inv_call_03_the_end_of_a_call_restores_nothing_for_a_place_that_kept_playing() {
    let rig = Rig::new();
    let (place, slot) = rig.started();
    rig.external(true);
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));

    rig.external(false);
    rig.settled();
    rig.settled();

    assert_eq!(rig.plays(&slot), 1, "the release resumed the place");
    place.update(PlaceInput { visible: true, ..rig.playing() }).unwrap();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 2));
}

#[test]
fn inv_call_04_a_suspension_released_during_a_call_restores_nothing() {
    let rig = Rig::new();
    let suspension = rig.layer.suspend_playback(Vec::new()).unwrap();
    rig.settled();
    let (place, slot) = {
        let place = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
        place.update(rig.playing()).unwrap();
        (place, rig.slot())
    };
    rig.external(true);

    suspension.release();
    rig.settled();
    place.update(PlaceInput { muted: Some(true), ..rig.playing() }).unwrap();
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Mute)));

    assert_eq!(rig.plays(&slot), 0, "the suspension release played during a call: {:?}", slot.calls());
}

#[test]
fn inv_call_01_the_start_of_a_call_hands_play_to_no_other_waiting_key() {
    let rig = Rig::new();
    let waiting = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    waiting.update(rig.playing()).unwrap();
    let slot = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
    let newer = rig.layer.open_place(MediaItem::bare_video(OTHER)).unwrap();
    newer.update(PlaceInput { visible: true, ..rig.playing() }).unwrap();
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));

    rig.external(true);
    rig.settled();
    waiting.update(PlaceInput { muted: Some(true), ..rig.playing() }).unwrap();
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Mute)));

    assert_eq!(rig.plays(&slot), 1, "the call handed play to the waiting key: {:?}", slot.calls());
}

#[test]
fn inv_call_03_closing_a_sibling_place_after_the_call_restores_nothing() {
    let rig = Rig::new();
    let (place, slot) = rig.started();
    let sibling = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    sibling.update(rig.playing()).unwrap();
    rig.external(true);
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));

    rig.external(false);
    sibling.release();
    place.update(PlaceInput { muted: Some(true), ..PlaceInput::default() }).unwrap();
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Mute)));

    assert_eq!(rig.plays(&slot), 1, "closing a sibling resumed the key: {:?}", slot.calls());
}

#[test]
fn inv_call_14_a_peek_declared_during_a_call_gives_no_play() {
    let rig = Rig::new();
    rig.external(true);
    let place = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    place.update(PlaceInput { peeking: true, ..PlaceInput::default() }).unwrap();
    let slot = rig.slot();
    rig.settled();
    place.update(PlaceInput { peeking: true, view_token: Some(7), ..PlaceInput::default() }).unwrap();
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Attach { view_token: 7 })));

    assert_eq!(rig.plays(&slot), 0, "a peek played during a call: {:?}", slot.calls());
}

#[test]
fn inv_call_14_the_start_of_a_call_pauses_a_running_peek() {
    let rig = Rig::new();
    let place = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    place.update(PlaceInput { peeking: true, ..PlaceInput::default() }).unwrap();
    let slot = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));

    rig.external(true);

    rig.caller.block_on(eventually(|| {
        let calls = slot.calls();
        calls.iter().rposition(|call| *call == SlotCall::Play).is_some_and(|play| calls[play..].contains(&SlotCall::Pause))
    }));
}

#[test]
fn inv_call_05_play_url_is_refused_during_a_call_and_answers_after_the_last_call_ends() {
    let rig = Rig::new();
    rig.audio(AudioInput::CallStarted { id: 1, kind: CallKind::Unmanaged, speaker: false });
    rig.audio(AudioInput::CallStarted { id: 2, kind: CallKind::Unmanaged, speaker: false });
    let item = MediaItem::bare_video(URL);

    assert!(matches!(rig.caller.block_on(rig.layer.play_url(item.clone())), Err(MediaError::InCall)));
    rig.audio(AudioInput::CallEnded { id: 1 });
    rig.audio(AudioInput::CallEnded { id: 1 });
    assert!(matches!(rig.caller.block_on(rig.layer.play_url(item.clone())), Err(MediaError::InCall)), "one call left, the call runs");
    rig.audio(AudioInput::CallEnded { id: 2 });

    assert!(rig.caller.block_on(rig.layer.play_url(item)).is_ok());
}

#[test]
fn inv_call_07_a_call_takes_its_place_in_the_arrival_order() {
    let rig = Rig::new();
    let place = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    place.update(rig.playing()).unwrap();
    let slot = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
    rig.external(true);
    place.update(PlaceInput { visible: true, ..rig.playing() }).unwrap();

    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));
    rig.settled();

    assert_eq!(rig.plays(&slot), 1, "an update accepted after the call started played");
}

#[test]
fn inv_audio_29_an_interruption_pauses_the_playing_key_and_its_resumable_end_plays_it_again() {
    let rig = Rig::new();
    let (place, slot) = rig.started();
    rig.audio(AudioInput::Audible { slot: 0, on: true });
    assert_eq!(rig.audio(AudioInput::InterruptionBegan), vec![AudioAction::Availability { value: MicrophoneAvailability::OtherApp }], "the pause goes to the core, not to the platform");
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));
    place.update(PlaceInput { visible: true, ..rig.playing() }).unwrap();
    rig.settled();
    assert_eq!(rig.plays(&slot), 1, "a declaration during the interruption played");
    rig.audio(AudioInput::Audible { slot: 0, on: false });

    rig.audio(AudioInput::InterruptionEnded { should_resume: true });

    rig.caller.block_on(eventually(|| rig.plays(&slot) == 2));
}

#[test]
fn inv_audio_30_a_pause_without_resume_forgets_and_only_a_new_declaration_plays() {
    let rig = Rig::new();
    let (place, slot) = rig.started();
    rig.audio(AudioInput::Audible { slot: 0, on: true });
    rig.audio(AudioInput::InterruptionBegan);
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));
    rig.audio(AudioInput::Audible { slot: 0, on: false });

    rig.audio(AudioInput::InterruptionEnded { should_resume: false });
    rig.settled();
    rig.settled();
    assert_eq!(rig.plays(&slot), 1, "the end resumed the key");

    place.update(PlaceInput { visible: true, ..rig.playing() }).unwrap();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 2));
}

#[test]
fn inv_audio_31_a_recording_keeps_playback_paused_until_it_ends_and_resumes_nothing() {
    let rig = Rig::new();
    let (place, slot) = rig.started();
    let playback = SessionWant::Playback { focus: AudioFocus::Exclusive };
    assert_eq!(rig.audio(AudioInput::Audible { slot: 0, on: true }), vec![AudioAction::Apply { generation: 1, want: playback, activate: true }]);
    let recording = SessionWant::Recording { kind: RecordingKind::Voice };
    assert_eq!(rig.audio(AudioInput::Capture { id: 1, kind: RecordingKind::Voice }), vec![AudioAction::Apply { generation: 2, want: recording, activate: true }]);
    assert_eq!(rig.audio(AudioInput::Applied { generation: 2, result: AppliedResult::Ok }), vec![AudioAction::Admit { id: 1 }]);
    rig.caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Pause)));
    place.update(PlaceInput { visible: true, ..rig.playing() }).unwrap();
    rig.settled();
    assert_eq!(rig.plays(&slot), 1, "a declaration during the recording played");
    rig.audio(AudioInput::Audible { slot: 0, on: false });

    assert_eq!(rig.audio(AudioInput::CaptureEnded { id: 1 }), vec![AudioAction::Deactivate { generation: 3 }]);
    rig.settled();
    rig.settled();
    assert_eq!(rig.plays(&slot), 1, "the end of the recording resumed the key");

    place.update(PlaceInput { visible: true, ..rig.playing() }).unwrap();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 2));
}

#[test]
fn inv_audio_44_audio_inputs_outlive_the_layer_handle_and_answer_closed_after_it_closes() {
    let rig = Rig::new();
    let inputs = rig.layer.audio_inputs();
    let actions = inputs.send(AudioInput::Capture { id: 1, kind: RecordingKind::Voice }).unwrap();
    let [AudioAction::Apply { generation, want: SessionWant::Recording { kind: RecordingKind::Voice }, activate: true }] = actions[..] else { panic!("{actions:?}") };
    rig.layer.shutdown();
    rig.caller.block_on(rig.layer.await_closed());

    assert!(matches!(inputs.send(AudioInput::Applied { generation, result: AppliedResult::Ok }), Err(MediaError::Closed)), "an input that would admit the recording");
    assert!(matches!(inputs.send(AudioInput::Audible { slot: 0, on: true }), Err(MediaError::Closed)), "an input that would pause through the core");
    assert!(matches!(inputs.send(AudioInput::Foreground { on: true }), Err(MediaError::Closed)), "an input without a core command");
    let Rig { layer, .. } = rig;
    drop(layer);
    assert!(matches!(inputs.send(AudioInput::Foreground { on: false }), Err(MediaError::Closed)), "after the last layer handle is gone");
}
