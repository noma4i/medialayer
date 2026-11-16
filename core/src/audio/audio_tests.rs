use super::*;
use AudioAction as A;
use AudioInput as I;

struct World {
    owner: AudioOwner,
}

impl World {
    fn new() -> Self {
        World { owner: AudioOwner::new(AudioFocus::Exclusive) }
    }

    fn with(focus: AudioFocus) -> Self {
        World { owner: AudioOwner::new(focus) }
    }

    fn playing() -> Self {
        let mut world = World::new();
        world.settle(I::Audible { slot: 0, on: true });
        world
    }

    fn raw(&mut self, input: AudioInput) -> Vec<AudioAction> {
        self.owner.step(input)
    }

    fn send(&mut self, input: AudioInput) -> Vec<AudioAction> {
        plain(self.raw(input))
    }

    fn settle(&mut self, input: AudioInput) -> Vec<AudioAction> {
        let mut chunk = self.raw(input);
        let mut all = Vec::new();
        loop {
            let last = chunk.iter().rev().find_map(generation_of);
            all.extend(chunk);
            let Some(generation) = last else { break };
            chunk = self.raw(I::Applied { generation, result: AppliedResult::Ok });
        }
        plain(all)
    }
}

fn generation_of(action: &AudioAction) -> Option<u64> {
    match action {
        A::Apply { generation, .. } | A::Deactivate { generation } => Some(*generation),
        _ => None,
    }
}

fn plain(actions: Vec<AudioAction>) -> Vec<AudioAction> {
    actions.into_iter().map(|action| match action {
        A::Apply { want, activate, .. } => A::Apply { generation: 0, want, activate },
        A::Deactivate { .. } => A::Deactivate { generation: 0 },
        other => other,
    }).collect()
}

fn apply(want: SessionWant) -> AudioAction {
    A::Apply { generation: 0, want, activate: true }
}

fn configure(want: SessionWant) -> AudioAction {
    A::Apply { generation: 0, want, activate: false }
}

const DEACTIVATE: AudioAction = A::Deactivate { generation: 0 };
const PLAYBACK: SessionWant = SessionWant::Playback { focus: AudioFocus::Exclusive };

fn available(value: MicrophoneAvailability) -> AudioAction {
    A::Availability { value }
}

fn own_callkit() -> CallKind {
    CallKind::CallKit { uuid: "own".into() }
}

#[test]
fn inv_audio_13_playback_holds_the_session_while_a_slot_sounds_and_releases_it_after_the_last() {
    for focus in [AudioFocus::Exclusive, AudioFocus::Duck, AudioFocus::Mix] {
        let mut world = World::with(focus);
        assert_eq!(world.settle(I::Audible { slot: 0, on: true }), vec![apply(SessionWant::Playback { focus })], "{focus:?}");
        assert_eq!(world.send(I::Audible { slot: 1, on: true }), vec![]);
        assert_eq!(world.send(I::Audible { slot: 0, on: false }), vec![]);
        assert_eq!(world.settle(I::Audible { slot: 1, on: false }), vec![DEACTIVATE], "{focus:?}");
    }
    assert_eq!(World::playing().send(I::Foreground { on: false }), vec![], "sounding audio keeps its session in the background");
}

#[test]
fn inv_audio_14_an_interruption_without_an_end_is_left_by_the_return_to_the_foreground() {
    let mut world = World::playing();
    assert_eq!(world.send(I::InterruptionBegan), vec![A::PauseAll { reason: PauseReason::Interrupted }, available(MicrophoneAvailability::OtherApp)]);
    assert_eq!(world.send(I::Audible { slot: 0, on: false }), vec![]);
    assert_eq!(world.send(I::Capture { id: 1, kind: RecordingKind::Voice }), vec![A::Refuse { id: 1, reason: RecordingRefusal::Interrupted }]);
    assert_eq!(world.send(I::Foreground { on: false }), vec![]);
    assert_eq!(world.send(I::Foreground { on: true }), vec![A::ForgetPaused, available(MicrophoneAvailability::Available)]);
    assert_eq!(world.settle(I::Capture { id: 2, kind: RecordingKind::Voice }), vec![apply(SessionWant::Recording { kind: RecordingKind::Voice }), A::Admit { id: 2 }]);
}

#[test]
fn inv_audio_15_the_end_of_an_interruption_resumes_only_when_the_system_says_so() {
    let mut world = World::playing();
    let pause = vec![A::PauseAll { reason: PauseReason::Interrupted }, available(MicrophoneAvailability::OtherApp)];
    for (end, resumes) in [
        (I::InterruptionEnded { should_resume: true }, true),
        (I::InterruptionEnded { should_resume: false }, false),
        (I::ResumptionRecommendation { should_resume: true }, true),
    ] {
        let label = format!("{end:?}");
        assert_eq!(world.send(I::InterruptionBegan), pause, "{label}");
        assert_eq!(world.send(I::Audible { slot: 0, on: false }), vec![]);
        if resumes {
            assert_eq!(world.settle(end), vec![apply(PLAYBACK), A::ResumePaused, available(MicrophoneAvailability::Available)], "{label}: the session comes back before the players");
            assert_eq!(world.send(I::Audible { slot: 0, on: true }), vec![], "{label}");
        } else {
            assert_eq!(world.send(end), vec![A::ForgetPaused, available(MicrophoneAvailability::Available)], "{label}");
            assert_eq!(world.settle(I::Audible { slot: 0, on: true }), vec![apply(PLAYBACK)], "{label}: a new intent brings the session back");
        }
    }
}

#[test]
fn inv_audio_16_a_lost_focus_is_a_state_the_library_does_not_fight() {
    let mut world = World::playing();
    assert_eq!(world.send(I::Focus { change: FocusChange::Loss }), vec![A::PauseAll { reason: PauseReason::FocusLost }, A::ForgetPaused, DEACTIVATE]);
    assert_eq!(world.send(I::Audible { slot: 0, on: false }), vec![]);
    for input in [I::Route { reason: RouteReason::NewDeviceAvailable, had_private_output: false }, I::Foreground { on: true }, I::Focus { change: FocusChange::Gain }] {
        let label = format!("{input:?}");
        assert_eq!(world.send(input), vec![], "{label}: no request without a new intent");
    }
    assert_eq!(world.settle(I::Audible { slot: 0, on: true }), vec![apply(PLAYBACK)]);
}

#[test]
fn inv_audio_17_a_transient_loss_pauses_until_the_gain_and_a_duck_lowers_the_volume() {
    let mut world = World::playing();
    assert_eq!(world.send(I::Focus { change: FocusChange::LossTransient }), vec![A::PauseAll { reason: PauseReason::Interrupted }]);
    assert_eq!(world.send(I::Audible { slot: 0, on: false }), vec![], "the focus request stays while paused");
    assert_eq!(world.send(I::Focus { change: FocusChange::Gain }), vec![A::ResumePaused], "the request was kept, the session stays until the players report");
    assert_eq!(world.send(I::Audible { slot: 0, on: true }), vec![]);
    assert_eq!(world.send(I::Focus { change: FocusChange::LossTransientDuck }), vec![A::VolumeMultiplier { value: 0.2 }]);
    assert_eq!(world.send(I::Focus { change: FocusChange::Gain }), vec![A::VolumeMultiplier { value: 1.0 }]);
}

#[test]
fn inv_audio_18_a_focus_request_refused_in_the_background_waits_for_the_foreground() {
    let mut world = World::new();
    assert_eq!(world.send(I::Foreground { on: false }), vec![]);
    assert_eq!(world.send(I::Audible { slot: 0, on: true }), vec![apply(PLAYBACK)]);
    assert_eq!(world.send(I::FocusRequest { answer: FocusAnswer::Failed }), vec![A::PauseAll { reason: PauseReason::Interrupted }]);
    assert_eq!(world.send(I::Audible { slot: 0, on: false }), vec![]);
    assert_eq!(world.send(I::Foreground { on: true }), vec![apply(PLAYBACK), A::ResumePaused]);
    assert_eq!(world.send(I::Audible { slot: 0, on: true }), vec![]);
    assert_eq!(World::new().send(I::FocusRequest { answer: FocusAnswer::Delayed }), vec![], "a delayed request waits for the gain");
}

#[test]
fn inv_audio_19_a_lost_private_output_pauses_and_the_routes_of_an_own_change_are_not_a_loss() {
    let mut world = World::playing();
    assert_eq!(world.send(I::Capture { id: 1, kind: RecordingKind::Video }), vec![A::PauseAll { reason: PauseReason::Capture }, apply(SessionWant::Recording { kind: RecordingKind::Video }), A::Admit { id: 1 }]);
    assert_eq!(world.send(I::Route { reason: RouteReason::OldDeviceUnavailable, had_private_output: true }), vec![], "a route inside the own change is ours");
    assert_eq!(world.send(I::Audible { slot: 0, on: false }), vec![]);
    assert_eq!(world.settle(I::CaptureEnded { id: 1 }), vec![A::ForgetPaused, DEACTIVATE], "the end of the recording resumes nothing");
    assert_eq!(world.settle(I::Audible { slot: 0, on: true }), vec![apply(PLAYBACK)]);
    let lost = vec![A::PauseAll { reason: PauseReason::OutputLost }, A::ForgetPaused];
    for (reason, private, expected) in [
        (RouteReason::OldDeviceUnavailable, true, lost.clone()),
        (RouteReason::OldDeviceUnavailable, false, vec![]),
        (RouteReason::CategoryChange, true, vec![]),
        (RouteReason::Override, true, vec![]),
        (RouteReason::NewDeviceAvailable, true, vec![]),
    ] {
        assert_eq!(world.send(I::Route { reason, had_private_output: private }), expected, "{reason:?} private={private}");
    }
    assert_eq!(world.send(I::Noisy), lost);
}

#[test]
fn inv_audio_20_a_recording_is_admitted_alone_pauses_playback_and_does_not_resume_it() {
    let mut world = World::playing();
    assert_eq!(world.settle(I::Capture { id: 1, kind: RecordingKind::Voice }), vec![A::PauseAll { reason: PauseReason::Capture }, apply(SessionWant::Recording { kind: RecordingKind::Voice }), A::Admit { id: 1 }]);
    assert_eq!(world.send(I::Capture { id: 2, kind: RecordingKind::Video }), vec![A::Refuse { id: 2, reason: RecordingRefusal::MicrophoneBusy }]);
    assert_eq!(world.send(I::Audible { slot: 0, on: false }), vec![]);
    assert_eq!(world.send(I::InterruptionBegan), vec![A::Revoke { id: 1, reason: RecordingRefusal::Interrupted }, A::ForgetPaused, available(MicrophoneAvailability::OtherApp)]);
    assert_eq!(world.send(I::InterruptionEnded { should_resume: true }), vec![available(MicrophoneAvailability::Available)], "the end of an interruption never resumes what the recording paused");
    assert_eq!(world.settle(I::Capture { id: 3, kind: RecordingKind::Voice }), vec![apply(SessionWant::Recording { kind: RecordingKind::Voice }), A::Admit { id: 3 }]);
    assert_eq!(world.settle(I::CaptureEnded { id: 3 }), vec![DEACTIVATE]);
}

#[test]
fn inv_audio_21_every_loss_of_the_microphone_revokes_the_running_recording_with_its_reason() {
    let revoke = |reason| A::Revoke { id: 1, reason };
    let cases: Vec<(AudioInput, Vec<AudioAction>)> = vec![
        (I::InterruptionBegan, vec![revoke(RecordingRefusal::Interrupted), available(MicrophoneAvailability::OtherApp)]),
        (I::Focus { change: FocusChange::Loss }, vec![revoke(RecordingRefusal::Interrupted), DEACTIVATE]),
        (I::Focus { change: FocusChange::LossTransient }, vec![revoke(RecordingRefusal::Interrupted), DEACTIVATE]),
        (I::Silenced { id: 1 }, vec![revoke(RecordingRefusal::MicrophoneSilent), DEACTIVATE]),
        (I::PhoneCall { active: true }, vec![revoke(RecordingRefusal::InCall), A::CallHold { on: true }, DEACTIVATE, available(MicrophoneAvailability::ExternalCall)]),
        (I::ObservedCalls { uuids: vec!["cellular".into()] }, vec![revoke(RecordingRefusal::InCall), A::CallHold { on: true }, DEACTIVATE, available(MicrophoneAvailability::ExternalCall)]),
        (I::CallStarted { id: 7, kind: CallKind::Unmanaged, speaker: false }, vec![revoke(RecordingRefusal::InCall), A::CallHold { on: true }, apply(SessionWant::Call { kind: CallKind::Unmanaged, speaker: false }), A::CallState { id: 7, state: CallState::Connecting }, available(MicrophoneAvailability::OwnCall)]),
        (I::Permission { granted: false }, vec![revoke(RecordingRefusal::PermissionDenied), DEACTIVATE, available(MicrophoneAvailability::PermissionDenied)]),
        (I::MediaReset, vec![revoke(RecordingRefusal::Interrupted)]),
    ];
    for (input, expected) in cases {
        let mut world = World::new();
        world.settle(I::Capture { id: 1, kind: RecordingKind::Video });
        let label = format!("{input:?}");
        assert_eq!(world.send(input), expected, "{label}");
        assert_eq!(world.send(I::CaptureEnded { id: 1 }), vec![], "{label}: the revoked recording is already gone");
    }
}

#[test]
fn inv_audio_22_an_own_call_takes_one_core_hold_and_its_own_mode_is_never_an_external_call() {
    let mut world = World::playing();
    let call = SessionWant::Call { kind: CallKind::Unmanaged, speaker: true };
    assert_eq!(world.settle(I::CallStarted { id: 7, kind: CallKind::Unmanaged, speaker: true }), vec![A::CallHold { on: true }, apply(call), A::CallState { id: 7, state: CallState::Connecting }, available(MicrophoneAvailability::OwnCall), A::CallState { id: 7, state: CallState::Live }]);
    assert_eq!(world.send(I::Audible { slot: 0, on: false }), vec![]);
    assert_eq!(world.send(I::CommunicationMode { active: true }), vec![]);
    assert_eq!(world.settle(I::CallEnded { id: 7 }), vec![A::CallHold { on: false }, DEACTIVATE, A::CallState { id: 7, state: CallState::Ended }, available(MicrophoneAvailability::Available)], "free at once, before the mode callback");
    assert_eq!(world.send(I::CommunicationMode { active: true }), vec![], "the late mode of the own call is ours");
    assert_eq!(world.send(I::CommunicationMode { active: false }), vec![]);
    assert_eq!(world.send(I::CommunicationMode { active: true }), vec![A::CallHold { on: true }, available(MicrophoneAvailability::ExternalCall)], "a later foreign mode is external");
}

#[test]
fn inv_audio_23_an_external_call_holds_the_core_until_its_platform_signal_ends() {
    for (begin, end) in [
        (I::ObservedCalls { uuids: vec!["cellular".into()] }, I::ObservedCalls { uuids: vec![] }),
        (I::PhoneCall { active: true }, I::PhoneCall { active: false }),
        (I::CommunicationMode { active: true }, I::CommunicationMode { active: false }),
    ] {
        let mut world = World::playing();
        let label = format!("{begin:?}");
        assert_eq!(world.send(begin), vec![A::CallHold { on: true }, available(MicrophoneAvailability::ExternalCall)], "{label}");
        assert_eq!(world.send(I::Capture { id: 1, kind: RecordingKind::Voice }), vec![A::Refuse { id: 1, reason: RecordingRefusal::InCall }], "{label}");
        assert_eq!(world.send(end), vec![A::CallHold { on: false }, available(MicrophoneAvailability::Available)], "{label}");
    }
    let mut world = World::new();
    assert_eq!(world.settle(I::CallStarted { id: 7, kind: own_callkit(), speaker: false }), vec![A::CallHold { on: true }, configure(SessionWant::Call { kind: own_callkit(), speaker: false }), A::CallState { id: 7, state: CallState::Connecting }, available(MicrophoneAvailability::OwnCall)]);
    assert_eq!(world.send(I::ObservedCalls { uuids: vec!["own".into()] }), vec![], "the own CallKit call seen by the observer is not external");
}

#[test]
fn inv_audio_24_a_callkit_call_releases_the_session_and_lets_the_system_activate_it() {
    let mut world = World::playing();
    let call = SessionWant::Call { kind: own_callkit(), speaker: false };
    let state = |state| A::CallState { id: 7, state };
    assert_eq!(world.send(I::CallStarted { id: 7, kind: own_callkit(), speaker: false }), vec![A::CallHold { on: true }, DEACTIVATE, configure(call), state(CallState::Connecting), available(MicrophoneAvailability::OwnCall)]);
    assert_eq!(world.send(I::Audible { slot: 0, on: false }), vec![]);
    assert_eq!(world.send(I::CallSystemActivated { id: 7 }), vec![state(CallState::Live)]);
    assert_eq!(world.send(I::InterruptionBegan), vec![state(CallState::Interrupted)]);
    assert_eq!(world.send(I::CallSystemActivated { id: 7 }), vec![state(CallState::Live)]);
    assert_eq!(world.send(I::CallSystemDeactivated { id: 7 }), vec![state(CallState::Interrupted)]);
    assert_eq!(world.settle(I::CallEnded { id: 7 }), vec![A::CallHold { on: false }, DEACTIVATE, state(CallState::Ended), available(MicrophoneAvailability::Available)]);
}

#[test]
fn inv_audio_25_a_telecom_call_leaves_focus_mode_and_route_to_telecom() {
    let mut world = World::playing();
    let state = |state| A::CallState { id: 7, state };
    assert_eq!(world.send(I::CallStarted { id: 7, kind: CallKind::Telecom, speaker: true }), vec![A::CallHold { on: true }, DEACTIVATE, state(CallState::Connecting), available(MicrophoneAvailability::OwnCall)]);
    assert_eq!(world.send(I::Audible { slot: 0, on: false }), vec![]);
    assert_eq!(world.send(I::CallSpeaker { id: 7, speaker: false }), vec![]);
    assert_eq!(world.send(I::CallSystemActivated { id: 7 }), vec![state(CallState::Live)]);
    assert_eq!(world.send(I::CallSystemDeactivated { id: 7 }), vec![state(CallState::Interrupted)]);
    assert_eq!(world.send(I::CallEnded { id: 7 }), vec![A::CallHold { on: false }, state(CallState::Ended), available(MicrophoneAvailability::Available)]);
}

#[test]
fn inv_audio_26_a_busy_release_is_finished_by_the_next_event_without_a_timer() {
    let mut world = World::playing();
    let idle = world.raw(I::Audible { slot: 0, on: false });
    assert_eq!(plain(idle.clone()), vec![DEACTIVATE]);
    let generation = idle.iter().find_map(generation_of).unwrap();
    assert_eq!(world.send(I::Applied { generation, result: AppliedResult::Busy }), vec![]);
    assert_eq!(world.send(I::Route { reason: RouteReason::CategoryChange, had_private_output: false }), vec![DEACTIVATE]);
}

#[test]
fn inv_audio_27_a_media_reset_drops_every_player_and_waits_for_a_new_intent() {
    let mut world = World::playing();
    world.send(I::InterruptionBegan);
    assert_eq!(world.send(I::Audible { slot: 0, on: false }), vec![]);
    assert_eq!(world.send(I::MediaReset), vec![A::ForgetPaused, available(MicrophoneAvailability::Available)]);
    assert_eq!(world.settle(I::Audible { slot: 0, on: true }), vec![apply(PLAYBACK)]);
    assert_eq!(World::playing().send(I::MediaReset), vec![A::PauseAll { reason: PauseReason::MediaReset }, A::ForgetPaused]);
}

#[test]
fn inv_audio_28_availability_is_one_value_derived_from_every_input() {
    let mut world = World::new();
    assert_eq!(world.send(I::Permission { granted: false }), vec![available(MicrophoneAvailability::PermissionDenied)]);
    assert_eq!(world.send(I::Capture { id: 1, kind: RecordingKind::Voice }), vec![A::Refuse { id: 1, reason: RecordingRefusal::PermissionDenied }]);
    assert_eq!(world.send(I::Permission { granted: true }), vec![available(MicrophoneAvailability::Available)]);
    assert_eq!(world.send(I::OtherRecording { active: true }), vec![available(MicrophoneAvailability::OtherApp)]);
    assert_eq!(world.send(I::Capture { id: 2, kind: RecordingKind::Voice }), vec![A::Refuse { id: 2, reason: RecordingRefusal::MicrophoneBusy }]);
    assert_eq!(world.send(I::OtherRecording { active: false }), vec![available(MicrophoneAvailability::Available)]);
    let call = SessionWant::Call { kind: CallKind::Unmanaged, speaker: false };
    assert_eq!(world.settle(I::CallStarted { id: 7, kind: CallKind::Unmanaged, speaker: false }), vec![A::CallHold { on: true }, apply(call), A::CallState { id: 7, state: CallState::Connecting }, available(MicrophoneAvailability::OwnCall), A::CallState { id: 7, state: CallState::Live }]);
    assert_eq!(world.send(I::PhoneCall { active: true }), vec![], "an own call stays OwnCall while an external one arrives");
    assert_eq!(world.settle(I::CallEnded { id: 7 }), vec![DEACTIVATE, A::CallState { id: 7, state: CallState::Ended }, available(MicrophoneAvailability::ExternalCall)]);
    assert_eq!(world.send(I::Permission { granted: true }), vec![], "an unchanged value is not sent again");
}
