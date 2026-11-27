use super::*;
use crate::api::test_support::{eventually, MockFactory, MockSlot};
use crate::api::{MediaError, MediaItem, PlayRefusal, PlaybackState, PlayerSlotFactory, SlotCall, Subscription};
use crate::audio::{AppliedResult, AudioAction, AudioInput, CallKind, PauseReason, RecordingKind};
use crate::ffi::{MediaLayer, MediaPlace, PlaybackListener, PlaybackSuspension};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use super::ingress_harness::*;

const URL: &str = "https://o.test/v1.mp4";

#[derive(Default)]
struct Seen(Mutex<Vec<PlaybackState>>, std::sync::atomic::AtomicUsize);

impl PlaybackListener for Seen {
    fn changed(&self, state: PlaybackState) {
        self.0.lock().unwrap().push(state);
        crate::test_signal::fire();
    }

    fn failed(&self, error: MediaError) {
        if matches!(error, MediaError::Closed) {
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl Seen {
    fn all(&self) -> Vec<PlaybackState> {
        self.0.lock().unwrap().clone()
    }

    fn last(&self) -> Option<PlaybackState> {
        self.0.lock().unwrap().last().cloned()
    }
}

fn state(playing: bool, wish: bool, reason: Option<PauseReason>, refused: Option<PlayRefusal>) -> PlaybackState {
    PlaybackState { playing, wish, reason, refused, attempt: 0 }
}

fn playing_state() -> PlaybackState {
    state(true, true, None, None)
}

struct Watched {
    place: Arc<MediaPlace>,
    seen: Arc<Seen>,
    _subscription: Arc<Subscription>,
}

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
        Rig::with(Arc::default())
    }

    fn with(factory: Arc<MockFactory>) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
        let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
        let window = layer.open_video_window().unwrap();
        window.replace(0, vec![Some(MediaItem::bare_video(URL))]).unwrap();
        let inputs = layer.audio_inputs();
        Rig { _dir: dir, layer, factory, caller: caller(), _window: window, inputs }
    }

    fn watched(&self, input: PlaceInput) -> Watched {
        let place = self.layer.open_place(MediaItem::bare_video(URL)).unwrap();
        let seen = Arc::new(Seen::default());
        let subscription = place.subscribe_playback(seen.clone()).unwrap();
        place.update(input).unwrap();
        Watched { place, seen, _subscription: subscription }
    }

    fn slot(&self) -> (u32, Arc<MockSlot>) {
        self.caller.block_on(eventually(|| self.factory.slot_holding(URL).is_some()));
        self.factory.slot_holding(URL).unwrap()
    }

    fn plays(&self, slot: &MockSlot) -> usize {
        slot.calls().iter().filter(|call| matches!(call, SlotCall::Play)).count()
    }

    fn transport(&self, slot: &MockSlot) -> Option<SlotCall> {
        slot.calls().into_iter().rev().find(|call| matches!(call, SlotCall::Play | SlotCall::Pause))
    }

    fn audio(&self, input: AudioInput) -> Vec<AudioAction> {
        self.inputs.send(input).unwrap()
    }

    fn record(&self) {
        let actions = self.audio(AudioInput::Capture { id: 1, kind: RecordingKind::Voice });
        let Some(generation) = actions.iter().find_map(|action| match action { AudioAction::Apply { generation, .. } => Some(*generation), _ => None }) else { panic!("the recording asks for its session: {actions:?}") };
        self.audio(AudioInput::Applied { generation, result: AppliedResult::Ok });
    }

    /// Everything declared before this call is applied when it returns.
    fn settled(&self) {
        let _ = self.caller.block_on(self.layer.placeholder(MediaItem::bare_video("https://o.test/barrier.mp4"), false));
    }

    fn until(&self, seen: &Seen, expected: &PlaybackState) {
        self.caller.block_on(eventually(|| seen.last().as_ref() == Some(expected)));
    }
}

#[derive(Debug, Clone, Copy)]
enum Pauser {
    Recording,
    Call,
    Interruption,
    OutputLoss,
    Suspension,
    Reset,
}

impl Pauser {
    fn inject(self, rig: &Rig) -> Option<Arc<PlaybackSuspension>> {
        match self {
            Pauser::Recording => rig.record(),
            Pauser::Call => drop(rig.audio(AudioInput::PhoneCall { active: true })),
            Pauser::Interruption => drop(rig.audio(AudioInput::InterruptionBegan)),
            Pauser::OutputLoss => drop(rig.audio(AudioInput::Noisy)),
            Pauser::Suspension => return Some(rig.layer.suspend_playback(Vec::new()).unwrap()),
            Pauser::Reset => drop(rig.audio(AudioInput::MediaReset)),
        }
        None
    }

    fn expected(self) -> PlaybackState {
        match self {
            Pauser::Recording => state(false, true, Some(PauseReason::Capture), None),
            Pauser::Call => state(false, false, Some(PauseReason::InCall), None),
            Pauser::Interruption => state(false, true, Some(PauseReason::Interrupted), None),
            Pauser::OutputLoss => state(false, false, Some(PauseReason::OutputLost), None),
            Pauser::Suspension => state(false, true, Some(PauseReason::Suspended), None),
            Pauser::Reset => state(false, false, Some(PauseReason::MediaReset), None),
        }
    }
}

const PAUSERS: [Pauser; 6] = [Pauser::Recording, Pauser::Call, Pauser::Interruption, Pauser::OutputLoss, Pauser::Suspension, Pauser::Reset];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Playing {
    Audible,
    Muted,
    Loading,
}

#[test]
fn inv_playback_01_a_user_play_and_pause_show_in_the_place_playback_state() {
    let rig = Rig::new();
    let watched = rig.watched(PlaceInput { playing: true, ..PlaceInput::default() });
    let (_, slot) = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
    rig.until(&watched.seen, &playing_state());

    watched.place.update(PlaceInput::default()).unwrap();

    rig.until(&watched.seen, &state(false, false, None, None));
    rig.caller.block_on(eventually(|| rig.transport(&slot) == Some(SlotCall::Pause)));
}

#[test]
fn inv_playback_02_every_pause_source_shows_paused_with_its_reason_for_audible_muted_and_loading_places() {
    for playing in [Playing::Audible, Playing::Muted, Playing::Loading] {
        for pauser in PAUSERS {
            let factory: Arc<MockFactory> = Arc::default();
            let held = (playing == Playing::Loading).then(|| factory.gate.hold());
            let rig = Rig::with(factory);
            let muted = (playing == Playing::Muted).then_some(true);
            let watched = rig.watched(PlaceInput { playing: true, muted, ..PlaceInput::default() });
            let loaded = (playing != Playing::Loading).then(|| {
                let (index, slot) = rig.slot();
                rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
                if playing == Playing::Audible {
                    rig.audio(AudioInput::Audible { slot: u64::from(index), on: true });
                }
                slot
            });
            if playing == Playing::Loading {
                rig.caller.block_on(eventually(|| watched.seen.last().is_some_and(|seen| seen.wish)));
            }

            let _suspension = pauser.inject(&rig);

            rig.until(&watched.seen, &pauser.expected());
            match (&loaded, pauser) {
                (Some(slot), Pauser::Reset) => rig.caller.block_on(eventually(|| slot.disposed.load(Ordering::SeqCst) == 1)),
                (Some(slot), _) => rig.caller.block_on(eventually(|| rig.transport(slot) == Some(SlotCall::Pause))),
                (None, _) => {
                    drop(held);
                    let (_, slot) = rig.slot();
                    rig.settled();
                    rig.settled();
                    assert_eq!(rig.plays(&slot), 0, "{playing:?} {pauser:?}: the loading place played after the pause: {:?}", slot.calls());
                }
            }
            assert_eq!(watched.seen.last(), Some(pauser.expected()), "{playing:?} {pauser:?}");
        }
    }
}

#[test]
fn inv_heal_01_a_user_play_after_an_interruption_without_end_plays() {
    let rig = Rig::new();
    let watched = rig.watched(PlaceInput { playing: true, ..PlaceInput::default() });
    let (index, slot) = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
    rig.audio(AudioInput::Audible { slot: u64::from(index), on: true });
    rig.audio(AudioInput::InterruptionBegan);
    rig.until(&watched.seen, &state(false, true, Some(PauseReason::Interrupted), None));
    rig.audio(AudioInput::Audible { slot: u64::from(index), on: false });
    watched.place.update(PlaceInput { playing: true, play_generation: 1, ..PlaceInput::default() }).unwrap();
    rig.until(&watched.seen, &playing_state());
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 2));
}

#[test]
fn inv_heal_audio_model() {
    use crate::audio::FocusChange;
    let rig = Rig::new();
    let watched = rig.watched(PlaceInput::default());
    let mut dice = crate::seeded::Dice::new("inv_heal_audio_model");
    for sequence in 0..500_u64 {
        for _ in 0..8 {
            let event = match dice.below(11) {
                0 => AudioInput::InterruptionBegan,
                1 => AudioInput::InterruptionEnded { should_resume: dice.percent(50) },
                2 => AudioInput::Focus { change: FocusChange::Loss },
                3 => AudioInput::Focus { change: FocusChange::LossTransient },
                4 => AudioInput::Focus { change: FocusChange::Gain },
                5 => AudioInput::CallStarted { id: 7, kind: CallKind::Unmanaged, speaker: false },
                6 => AudioInput::CallSystemActivated { id: 7 },
                7 => AudioInput::Capture { id: 1, kind: RecordingKind::Voice },
                8 => AudioInput::MediaReset,
                9 => AudioInput::Foreground { on: true },
                _ => AudioInput::Audible { slot: 0, on: true },
            };
            rig.audio(event);
        }
        rig.audio(AudioInput::CallEnded { id: 7 });
        rig.audio(AudioInput::CaptureEnded { id: 1 });
        watched.place.update(PlaceInput { playing: false, play_generation: sequence + 1, ..PlaceInput::default() }).unwrap();
        rig.settled();
        rig.caller.block_on(eventually(|| watched.seen.last().is_some_and(|state| !state.playing && !state.wish)));
        watched.place.update(PlaceInput { playing: true, play_generation: sequence + 1, ..PlaceInput::default() }).unwrap();
        rig.until(&watched.seen, &playing_state());
    }
}

#[test]
fn inv_playback_03_a_resumable_interruption_end_shows_the_place_playing_again() {
    let rig = Rig::new();
    let watched = rig.watched(PlaceInput { playing: true, ..PlaceInput::default() });
    let (index, slot) = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
    rig.audio(AudioInput::Audible { slot: u64::from(index), on: true });
    rig.audio(AudioInput::InterruptionBegan);
    rig.until(&watched.seen, &state(false, true, Some(PauseReason::Interrupted), None));
    rig.audio(AudioInput::Audible { slot: u64::from(index), on: false });

    let end = rig.audio(AudioInput::InterruptionEnded { should_resume: true });
    let Some(generation) = end.iter().find_map(|action| match action { AudioAction::Apply { generation, .. } => Some(*generation), _ => None }) else { panic!("the resumed demand asks for its session: {end:?}") };
    rig.audio(AudioInput::Applied { generation, result: AppliedResult::Ok });

    rig.until(&watched.seen, &playing_state());
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 2));
}

#[test]
fn inv_playback_04_a_forgotten_wish_ignores_a_stale_play_and_a_new_play_generation_plays() {
    let rig = Rig::new();
    let watched = rig.watched(PlaceInput { playing: true, ..PlaceInput::default() });
    let (index, slot) = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
    rig.audio(AudioInput::Audible { slot: u64::from(index), on: true });
    rig.audio(AudioInput::Noisy);
    let forgotten = state(false, false, Some(PauseReason::OutputLost), None);
    rig.until(&watched.seen, &forgotten);
    rig.audio(AudioInput::Audible { slot: u64::from(index), on: false });

    watched.place.update(PlaceInput { visible: true, playing: true, play_generation: 0, ..PlaceInput::default() }).unwrap();
    rig.settled();
    rig.settled();
    assert_eq!(rig.plays(&slot), 1, "a stale play of the forgotten generation played: {:?}", slot.calls());
    assert_eq!(watched.seen.last(), Some(forgotten));
    let mark = slot.calls().len();

    watched.place.update(PlaceInput { visible: true, playing: true, play_generation: 1, ..PlaceInput::default() }).unwrap();

    rig.until(&watched.seen, &playing_state());
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 2));
    rig.settled();
    assert_eq!(slot.calls()[mark..].to_vec(), vec![SlotCall::Play]);
}

#[test]
fn inv_playback_05_a_play_during_a_call_is_refused_and_a_play_during_a_recording_plays() {
    let rig = Rig::new();
    let (_, slot) = rig.slot();
    let _suspension = Pauser::Call.inject(&rig);
    rig.settled();
    let watched = rig.watched(PlaceInput { playing: true, play_generation: 1, ..PlaceInput::default() });
    rig.until(&watched.seen, &state(false, false, None, Some(PlayRefusal::InCall)));
    rig.settled();
    assert_eq!(rig.plays(&slot), 0, "the refused play reached the slot: {:?}", slot.calls());

    let rig = Rig::new();
    let (_, slot) = rig.slot();
    rig.record();
    rig.settled();
    let _watched = rig.watched(PlaceInput { playing: true, play_generation: 1, ..PlaceInput::default() });
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
}

#[test]
fn inv_playback_06_mute_rate_group_or_view_changes_after_a_forgotten_wish_play_nothing() {
    let changes: [(&str, PlaceInput, Option<SlotCall>); 4] = [
        ("mute", PlaceInput { muted: Some(true), ..PlaceInput::default() }, Some(SlotCall::Mute)),
        ("rate", PlaceInput { rate: Some(2.0), ..PlaceInput::default() }, Some(SlotCall::Rate { rate: 2.0 })),
        ("group", PlaceInput { group: Some("chat".to_string()), ..PlaceInput::default() }, None),
        ("view", PlaceInput { view_token: Some(7), ..PlaceInput::default() }, Some(SlotCall::Attach { view_token: 7 })),
    ];
    for (name, change, landed) in changes {
        let rig = Rig::new();
        let watched = rig.watched(PlaceInput { playing: true, ..PlaceInput::default() });
        let (index, slot) = rig.slot();
        rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
        rig.audio(AudioInput::Audible { slot: u64::from(index), on: true });
        rig.audio(AudioInput::Noisy);
        rig.until(&watched.seen, &state(false, false, Some(PauseReason::OutputLost), None));
        rig.audio(AudioInput::Audible { slot: u64::from(index), on: false });
        let before = watched.seen.all();

        watched.place.update(PlaceInput { playing: true, ..change }).unwrap();
        if let Some(call) = landed {
            rig.caller.block_on(eventually(|| slot.calls().contains(&call)));
        }
        rig.settled();
        rig.settled();

        assert_eq!(rig.plays(&slot), 1, "{name}: the change played the forgotten wish: {:?}", slot.calls());
        assert_eq!(watched.seen.all(), before, "{name}: the change altered the playback state");
    }
}

#[test]
fn inv_playback_07_a_play_url_player_is_paused_by_a_call_and_plays_on_an_explicit_play() {
    let rig = Rig::new();
    let player = rig.caller.block_on(rig.layer.play_url(MediaItem::bare_video(URL))).unwrap();
    let seen = Arc::new(Seen::default());
    let _subscription = player.subscribe_playback(seen.clone()).unwrap();
    player.play().unwrap();
    let (_, slot) = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
    rig.until(&seen, &playing_state());

    rig.audio(AudioInput::PhoneCall { active: true });
    rig.until(&seen, &state(false, false, Some(PauseReason::InCall), None));
    rig.caller.block_on(eventually(|| rig.transport(&slot) == Some(SlotCall::Pause)));
    rig.audio(AudioInput::PhoneCall { active: false });
    rig.settled();
    rig.settled();
    assert_eq!(rig.plays(&slot), 1, "the end of the call resumed the player: {:?}", slot.calls());

    player.play().unwrap();

    rig.until(&seen, &playing_state());
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 2));
}

#[test]
fn inv_playback_08_a_media_reset_disposes_the_native_player_and_a_new_play_builds_another() {
    let rig = Rig::new();
    let watched = rig.watched(PlaceInput { playing: true, ..PlaceInput::default() });
    let (index, old) = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&old) == 1));
    rig.audio(AudioInput::Audible { slot: u64::from(index), on: true });

    rig.audio(AudioInput::MediaReset);

    rig.until(&watched.seen, &state(false, false, Some(PauseReason::MediaReset), None));
    rig.caller.block_on(eventually(|| old.disposed.load(Ordering::SeqCst) == 1));
    watched.place.update(PlaceInput { playing: true, play_generation: 1, ..PlaceInput::default() }).unwrap();
    let fresh = || rig.factory.slots.lock().unwrap().values().find(|slot| !Arc::ptr_eq(slot, &old) && slot.loads() > 0 && rig.plays(slot) > 0).cloned();
    rig.caller.block_on(eventually(|| fresh().is_some()));
    let new = fresh().unwrap();
    rig.until(&watched.seen, &playing_state());
    rig.settled();

    assert_eq!((new.loads(), rig.plays(&new)), (1, 1), "the new play loads and plays a new player once: {:?}", new.calls());
    assert_eq!(rig.plays(&old), 1, "the disposed player played again: {:?}", old.calls());
}

#[test]
fn inv_playback_09_reading_a_route_for_a_frame_is_not_playback_and_works_during_a_call() {
    let rig = Rig::new();
    rig.audio(AudioInput::PhoneCall { active: true });
    rig.settled();

    let route = rig.caller.block_on(rig.layer.read_route(MediaItem::bare_video("https://o.test/frame.mp4")));

    assert!(route.as_ref().is_ok_and(|route| !route.url.is_empty()), "a frame read was refused like a play during the call: {route:?}");
}

#[test]
fn inv_playback_10_a_media_reset_rebuilds_the_player_with_the_declared_view_of_the_place() {
    let rig = Rig::new();
    let watched = rig.watched(PlaceInput { playing: true, view_token: Some(5), ..PlaceInput::default() });
    let (index, old) = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&old) == 1));
    rig.audio(AudioInput::Audible { slot: u64::from(index), on: true });

    rig.audio(AudioInput::MediaReset);
    rig.until(&watched.seen, &state(false, false, Some(PauseReason::MediaReset), None));
    watched.place.update(PlaceInput { playing: true, view_token: Some(5), play_generation: 1, ..PlaceInput::default() }).unwrap();
    let fresh = || rig.factory.slots.lock().unwrap().values().find(|slot| !Arc::ptr_eq(slot, &old) && rig.plays(slot) > 0).cloned();
    rig.caller.block_on(eventually(|| fresh().is_some()));

    let new = fresh().unwrap();
    assert!(new.calls().contains(&SlotCall::Attach { view_token: 5 }), "the reset lost the view of the place: {:?}", new.calls());
}

#[test]
fn inv_playback_11_the_session_changes_only_after_every_paused_slot_confirmed_its_pause() {
    use futures_util::FutureExt;
    let factory = Arc::<MockFactory>::default();
    let rig = Rig::with(factory.clone());
    let _watched = rig.watched(PlaceInput { playing: true, play_generation: 1, ..PlaceInput::default() });
    let (index, slot) = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
    rig.audio(AudioInput::Audible { slot: u64::from(index), on: true });
    let held = factory.pause_gate.hold();

    let actions = rig.audio(AudioInput::CallStarted { id: 1, kind: CallKind::Unmanaged, speaker: false });

    let barrier = actions.iter().position(|action| matches!(action, AudioAction::AwaitPaused { .. })).unwrap_or_else(|| panic!("the pause for the call is a barrier: {actions:?}"));
    let session = actions.iter().position(|action| matches!(action, AudioAction::Apply { .. } | AudioAction::Deactivate { .. })).unwrap_or_else(|| panic!("the call asks for its session: {actions:?}"));
    assert!(barrier < session, "the session step comes before the pause barrier: {actions:?}");
    let AudioAction::AwaitPaused { barrier } = actions[barrier] else { unreachable!() };
    rig.caller.block_on(eventually(|| factory.pause_gate.entered() >= 1));
    let mut paused = Box::pin(rig.inputs.paused(barrier));
    assert!((&mut paused).now_or_never().is_none(), "the barrier passed while the slot was still pausing");

    drop(held);

    rig.caller.block_on(paused);
    assert!(slot.calls().contains(&SlotCall::Pause), "{:?}", slot.calls());
}

#[test]
fn inv_playback_12_a_barrier_without_a_playing_slot_passes_at_once() {
    use futures_util::FutureExt;
    let rig = Rig::new();
    let actions = rig.audio(AudioInput::CallStarted { id: 1, kind: CallKind::Unmanaged, speaker: false });
    for barrier in actions.iter().filter_map(|action| match action { AudioAction::AwaitPaused { barrier } => Some(*barrier), _ => None }) {
        rig.settled();
        assert!(Box::pin(rig.inputs.paused(barrier)).now_or_never().is_some(), "nothing pauses, yet the barrier waits: {actions:?}");
    }
}

struct Released(Arc<Seen>);

impl PlaybackListener for Released {
    fn changed(&self, state: PlaybackState) { self.0.changed(state) }
    fn failed(&self, error: MediaError) {
        self.0.failed(error);
        crate::test_signal::fire();
    }
}

impl Drop for Released {
    fn drop(&mut self) { crate::test_signal::fire(); }
}

#[test]
fn inv_playback_13_releasing_a_place_ends_its_playback_subscriptions_and_drops_their_listeners() {
    let rig = Rig::new();
    let place = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    let seen = Arc::new(Seen::default());
    let listener = Arc::new(Released(seen.clone()));
    let held = Arc::downgrade(&listener);
    let _subscription = place.subscribe_playback(listener).unwrap();
    place.update(PlaceInput { playing: true, play_generation: 1, ..PlaceInput::default() }).unwrap();
    rig.until(&seen, &playing_state());
    rig.settled();
    let before = seen.all().len();

    place.release();

    rig.caller.block_on(eventually(|| seen.1.load(Ordering::SeqCst) == 1 && held.upgrade().is_none()));
    rig.settled();
    assert_eq!(seen.1.load(Ordering::SeqCst), 1, "the subscription did not end with Closed exactly once");
    assert_eq!(seen.all().len(), before, "a released place published a state");
}


fn listen(rig: &Rig, player: &crate::ffi::MediaPlayer) -> (Arc<Seen>, Arc<Subscription>) {
    let seen = Arc::new(Seen::default());
    let subscription = player.subscribe_playback(seen.clone()).unwrap();
    rig.settled();
    (seen, subscription)
}

#[test]
fn inv_playback_14_a_player_shows_paused_when_its_slot_ends_fails_or_is_released() {
    let ends: [(&str, fn(&Rig, u32, &MockSlot)); 4] = [
        ("ended", |rig, index, slot| { let events = rig.layer.player_events(); events.ready(index, slot.generation()); events.ended(index, slot.generation()) }),
        ("failed", |rig, index, slot| rig.layer.player_events().failed(index, slot.generation(), 500, "origin".to_string())),
        ("release all", |rig, _, _| rig.layer.release_all().unwrap()),
        ("background", |rig, _, _| rig.layer.player_events().background()),
    ];
    for (name, end) in ends {
        let rig = Rig::new();
        let url = "https://o.test/voice.m4a";
        let player = rig.caller.block_on(rig.layer.play_url(MediaItem::bare(url, crate::api::Kind::Audio))).unwrap();
        let (seen, _subscription) = listen(&rig, &player);
        player.play().unwrap();
        rig.caller.block_on(eventually(|| rig.factory.slot_holding(url).is_some()));
        let (index, slot) = rig.factory.slot_holding(url).unwrap();
        rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
        rig.until(&seen, &playing_state());

        end(&rig, index, &slot);

        rig.caller.block_on(eventually(|| seen.last().is_some_and(|state| !state.playing)));
        assert!(!seen.last().unwrap().playing, "{name}: the player still shows playing");
    }
}

#[test]
fn inv_playback_15_play_a_then_b_then_a_again_plays_a_and_shows_b_paused() {
    let rig = Rig::new();
    let (a_url, b_url) = ("https://o.test/a.mp4", "https://o.test/b.mp4");
    let a = rig.caller.block_on(rig.layer.play_url(MediaItem::bare_video(a_url))).unwrap();
    let b = rig.caller.block_on(rig.layer.play_url(MediaItem::bare_video(b_url))).unwrap();
    let (a_seen, _a_subscription) = listen(&rig, &a);
    let (b_seen, _b_subscription) = listen(&rig, &b);
    let playing = |seen: &Seen| seen.last().is_some_and(|state| state.playing);

    a.play().unwrap();
    rig.caller.block_on(eventually(|| playing(&a_seen)));
    b.play().unwrap();
    rig.caller.block_on(eventually(|| playing(&b_seen) && !playing(&a_seen)));

    a.play().unwrap();

    rig.caller.block_on(eventually(|| playing(&a_seen) && !playing(&b_seen)));
    let (_, a_slot) = rig.factory.slot_holding(a_url).unwrap();
    rig.caller.block_on(eventually(|| rig.transport(&a_slot) == Some(SlotCall::Play)));
    assert_eq!(rig.transport(&a_slot), Some(SlotCall::Play), "A shows playing but its slot did not play again: {:?}", a_slot.calls());
}

struct Blocking {
    log: Mutex<Vec<&'static str>>,
    entered: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    release: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
}

impl PlaybackListener for Blocking {
    fn changed(&self, _state: PlaybackState) {
        self.log.lock().unwrap().push("changed start");
        if let Some(entered) = self.entered.lock().unwrap().take() {
            entered.send(()).unwrap();
            let release = self.release.lock().unwrap().take().unwrap();
            release.recv().unwrap();
        }
        self.log.lock().unwrap().push("changed end");
        crate::test_signal::fire();
    }

    fn failed(&self, _error: MediaError) {
        self.log.lock().unwrap().push("failed");
        crate::test_signal::fire();
    }
}

#[test]
fn inv_playback_16_a_release_ends_the_subscription_only_after_a_running_change_and_nothing_follows() {
    let rig = Rig::new();
    let place = rig.layer.open_place(MediaItem::bare_video(URL)).unwrap();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let listener = Arc::new(Blocking { log: Mutex::new(Vec::new()), entered: Mutex::new(Some(entered_tx)), release: Mutex::new(Some(release_rx)) });
    let _subscription = place.subscribe_playback(listener.clone()).unwrap();
    entered_rx.recv().unwrap();

    place.release();
    place.update(PlaceInput { playing: true, ..PlaceInput::default() }).ok();
    rig.settled();
    assert_eq!(*listener.log.lock().unwrap(), vec!["changed start"], "the subscription ended while its change still ran");

    release_tx.send(()).unwrap();

    rig.caller.block_on(eventually(|| listener.log.lock().unwrap().last() == Some(&"failed")));
    rig.settled();
    assert_eq!(*listener.log.lock().unwrap(), vec!["changed start", "changed end", "failed"]);
}

#[test]
fn inv_playback_18_a_play_after_the_end_plays_the_same_player_again() {
    let rig = Rig::new();
    let url = "https://o.test/voice.m4a";
    let player = rig.caller.block_on(rig.layer.play_url(MediaItem::bare(url, crate::api::Kind::Audio))).unwrap();
    let (seen, _subscription) = listen(&rig, &player);
    player.play().unwrap();
    rig.caller.block_on(eventually(|| rig.factory.slot_holding(url).is_some()));
    let (index, slot) = rig.factory.slot_holding(url).unwrap();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
    let events = rig.layer.player_events();
    events.ready(index, slot.generation());
    events.ended(index, slot.generation());
    rig.caller.block_on(eventually(|| seen.last().is_some_and(|state| !state.playing)));

    player.play().unwrap();

    rig.until(&seen, &playing_state());
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 2 || rig.factory.slots.lock().unwrap().values().any(|other| !Arc::ptr_eq(other, &slot) && rig.plays(other) > 0)));
}

#[test]
fn inv_playback_17_an_output_loss_in_a_full_input_queue_still_forgets_the_wish_before_a_resume() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let pause = Pause::new();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), Some(IngressLimits { entries: 16, owners: 256, bytes: u64::MAX })), pause.clone()).unwrap();
    let caller = caller();
    let window = layer.open_video_window().unwrap();
    window.replace(0, vec![Some(MediaItem::bare_video(URL))]).unwrap();
    let inputs = layer.audio_inputs();
    let place = layer.open_place(MediaItem::bare_video(URL)).unwrap();
    let seen = Arc::new(Seen::default());
    let _subscription = place.subscribe_playback(seen.clone()).unwrap();
    place.update(PlaceInput { playing: true, play_generation: 1, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(URL).is_some()));
    let (index, slot) = factory.slot_holding(URL).unwrap();
    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Play)));
    inputs.send(AudioInput::Audible { slot: u64::from(index), on: true }).unwrap();
    caller.block_on(eventually(|| seen.last() == Some(playing_state())));
    let pads: Vec<Arc<MediaPlace>> = (0..32).map(|n| {
        let pad = layer.open_place(MediaItem::bare_video(&format!("https://o.test/pad{n}.mp4"))).unwrap();
        let _ = caller.block_on(layer.placeholder(MediaItem::bare_video("https://o.test/barrier.mp4"), false));
        pad
    }).collect();
    caller.block_on(layer.limits()).unwrap();
    pause.set(true);
    let mut full = false;
    for (n, pad) in pads.iter().enumerate() {
        if pad.update(PlaceInput { visible: true, view_token: Some(n as u64), ..PlaceInput::default() }).is_err() { full = true; break; }
    }
    assert!(full, "the input queue never filled");

    inputs.send(AudioInput::InterruptionBegan).unwrap();
    inputs.send(AudioInput::Noisy).unwrap();
    inputs.send(AudioInput::Audible { slot: u64::from(index), on: false }).unwrap();
    let end = inputs.send(AudioInput::InterruptionEnded { should_resume: true }).unwrap();
    for generation in end.iter().filter_map(|action| match action { AudioAction::Apply { generation, .. } => Some(*generation), _ => None }) {
        inputs.send(AudioInput::Applied { generation, result: AppliedResult::Ok }).unwrap();
    }
    pause.set(false);

    let forgotten = state(false, false, Some(PauseReason::OutputLost), None);
    caller.block_on(eventually(|| seen.last() == Some(forgotten.clone())));
    let _ = caller.block_on(layer.placeholder(MediaItem::bare_video("https://o.test/barrier.mp4"), false));
    assert_eq!(seen.last(), Some(forgotten), "the resume played the wish the output loss forgot");
    assert_eq!(slot.calls().iter().filter(|call| matches!(call, SlotCall::Play)).count(), 1, "the slot played again: {:?}", slot.calls());
}

fn audio_op(code: usize) -> AudioOp {
    match code {
        0 => AudioOp::PauseAll(PauseReason::Capture),
        1 => AudioOp::PauseAll(PauseReason::Interrupted),
        2 => AudioOp::PauseAll(PauseReason::FocusLost),
        3 => AudioOp::PauseAll(PauseReason::OutputLost),
        4 => AudioOp::PauseAll(PauseReason::MediaReset),
        5 => AudioOp::PauseAll(PauseReason::InCall),
        6 => AudioOp::PauseAll(PauseReason::Suspended),
        7 => AudioOp::ResumePaused,
        8 => AudioOp::ForgetPaused,
        9 => AudioOp::CallHold(true),
        _ => AudioOp::CallHold(false),
    }
}

fn after_audio(prefix: &[usize], run: Vec<AudioOp>) -> Vec<PlaybackState> {
    let rig = Rig::new();
    let places = [
        rig.watched(PlaceInput { playing: true, play_generation: 1, ..PlaceInput::default() }),
        rig.watched(PlaceInput { peeking: true, ..PlaceInput::default() }),
        rig.watched(PlaceInput::default()),
    ];
    let (_, slot) = rig.slot();
    rig.caller.block_on(eventually(|| rig.plays(&slot) == 1));
    let queue = rig.layer.queue();
    queue.submit_audio(prefix.iter().map(|code| Declared::Audio(CoreAudio::Op(audio_op(*code)))).collect()).unwrap();
    queue.submit_audio(run.into_iter().map(|op| Declared::Audio(CoreAudio::Op(op))).collect()).unwrap();
    rig.settled();
    places.iter().map(|watched| {
        let now = Arc::new(Seen::default());
        let _subscription = watched.place.subscribe_playback(now.clone()).unwrap();
        rig.caller.block_on(eventually(|| now.last().is_some()));
        now.all()[0].clone()
    }).collect()
}

#[test]
fn inv_ingress_56_a_folded_audio_run_leaves_every_place_as_the_original_run() {
    let mut dice = crate::seeded::Dice::new("inv_ingress_56_a_folded_audio_run_leaves_every_place_as_the_original_run");
    for _ in 0..24 {
        let prefix: Vec<usize> = (0..dice.index(4)).map(|_| dice.index(11)).collect();
        let run: Vec<usize> = (0..1 + dice.index(24)).map(|_| dice.index(11)).collect();
        let folded = super::audio_compose::compose(run.iter().map(|code| audio_op(*code)).collect());
        assert!(folded.len() <= run.len(), "the fold grew the run {run:?} to {}", folded.len());
        let original = after_audio(&prefix, run.iter().map(|code| audio_op(*code)).collect());
        assert_eq!(after_audio(&prefix, folded), original, "prefix {prefix:?}, run {run:?}");
    }
}
