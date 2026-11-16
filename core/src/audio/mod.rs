//! Focus transitions derive from AndroidX Media3 AudioFocusManager (Apache-2.0).
mod system;
mod types;
use statig::blocking::*;
use std::collections::BTreeSet;
use system::{current, Exit, System, SystemEvent, SystemMachine};
pub use types::*;

#[cfg(test)]
mod audio_tests;

#[derive(Clone)]
struct Call {
    id: u64,
    kind: CallKind,
    speaker: bool,
    state: CallState,
}

#[derive(Clone)]
pub struct AudioOwner {
    focus: AudioFocus,
    audible: BTreeSet<u64>,
    volume_readers: BTreeSet<u64>,
    captures: Vec<(u64, RecordingKind)>,
    calls: Vec<Call>,
    observed: BTreeSet<String>,
    phone: bool,
    comm_mode: bool,
    own_mode: bool,
    permission: bool,
    other_recording: bool,
    flowing: bool,
    system: StateMachine<SystemMachine>,
    foreground: bool,
    applied: Option<SessionWant>,
    inflight: Option<u64>,
    release_pending: bool,
    generation: u64,
    paused: Option<PauseReason>,
    resuming: bool,
    last_availability: MicrophoneAvailability,
    hold: bool,
}

impl AudioOwner {
    pub fn new(focus: AudioFocus) -> Self {
        Self {
            focus,
            audible: BTreeSet::new(),
            volume_readers: BTreeSet::new(),
            captures: Vec::new(),
            calls: Vec::new(),
            observed: BTreeSet::new(),
            phone: false,
            comm_mode: false,
            own_mode: false,
            permission: true,
            other_recording: false,
            flowing: true,
            system: SystemMachine.state_machine(),
            foreground: true,
            applied: None,
            inflight: None,
            release_pending: false,
            generation: 0,
            paused: None,
            resuming: false,
            last_availability: MicrophoneAvailability::Available,
            hold: false,
        }
    }

    fn system(&self) -> System {
        current(&self.system)
    }
    fn system_event(&mut self, event: SystemEvent, actions: &mut Vec<AudioAction>) {
        let mut exit = Exit::default();
        self.system.handle_with_context(&event, &mut exit);
        if let Some(resume) = exit.resume {
            self.finish_interruption(resume, actions);
        }
        if exit.restore_volume {
            actions.push(AudioAction::VolumeMultiplier { value: 1.0 });
        }
    }
    fn external(&self) -> bool {
        self.phone
            || (self.comm_mode && !self.own_mode)
            || self.observed.iter().any(|uuid| {
                !self.calls.iter().any(
                    |call| matches!(&call.kind, CallKind::CallKit { uuid: own } if own == uuid),
                )
            })
    }
    fn availability(&self) -> MicrophoneAvailability {
        if !self.permission {
            MicrophoneAvailability::PermissionDenied
        } else if !self.calls.is_empty() {
            MicrophoneAvailability::OwnCall
        } else if self.external() {
            MicrophoneAvailability::ExternalCall
        } else if self.other_recording || self.system() == System::Interrupted {
            MicrophoneAvailability::OtherApp
        } else {
            MicrophoneAvailability::Available
        }
    }
    fn want(&self) -> Option<SessionWant> {
        if let Some(call) = self.calls.last() {
            if call.kind != CallKind::Telecom && self.system() != System::Interrupted {
                return Some(SessionWant::Call {
                    kind: call.kind.clone(),
                    speaker: call.speaker,
                });
            }
        }
        if self.calls.iter().any(|call| call.kind == CallKind::Telecom) {
            return None;
        }
        if self.system() != System::Interrupted {
            if let Some((_, kind)) = self.captures.last() {
                return Some(SessionWant::Recording { kind: *kind });
            }
        }
        if (!self.audible.is_empty() || self.paused.is_some() || self.resuming)
            && matches!(
                self.system(),
                System::Free | System::Ducked | System::Transient
            )
        {
            return Some(SessionWant::Playback { focus: self.focus });
        }
        if !self.volume_readers.is_empty()
            && matches!(self.system(), System::Free | System::Ducked | System::Transient)
        {
            return Some(SessionWant::Playback { focus: AudioFocus::Mix });
        }
        None
    }
    fn revoke(&mut self, reason: RecordingRefusal, actions: &mut Vec<AudioAction>) {
        for (id, _) in self.captures.drain(..) {
            actions.push(AudioAction::Revoke { id, reason });
        }
    }
    fn pause(&mut self, reason: PauseReason, actions: &mut Vec<AudioAction>) {
        if !self.audible.is_empty() {
            actions.push(AudioAction::PauseAll { reason });
            if matches!(reason, PauseReason::Capture | PauseReason::Interrupted) {
                self.paused = Some(reason);
            } else {
                actions.push(AudioAction::ForgetPaused);
                self.paused = None;
            }
        }
    }
    fn finish_interruption(&mut self, resume: bool, actions: &mut Vec<AudioAction>) {
        if self.paused == Some(PauseReason::Interrupted) {
            self.resuming = resume;
            actions.push(if resume {
                AudioAction::ResumePaused
            } else {
                AudioAction::ForgetPaused
            });
            self.paused = None;
        }
    }
    fn call_state(&mut self, id: u64, state: CallState, actions: &mut Vec<AudioAction>) {
        if let Some(call) = self.calls.iter_mut().find(|call| call.id == id) {
            if call.state != state {
                call.state = state;
                actions.push(AudioAction::CallState { id, state });
            }
        }
    }
    fn deactivate(&mut self, actions: &mut Vec<AudioAction>) {
        self.generation += 1;
        self.inflight = Some(self.generation);
        self.applied = None;
        self.release_pending = false;
        actions.push(AudioAction::Deactivate {
            generation: self.generation,
        });
    }
    fn reconcile(&mut self, actions: &mut Vec<AudioAction>) {
        let want = self.want();
        if self.applied == want && !self.release_pending {
            return;
        }
        match want {
            None => self.deactivate(actions),
            Some(want) => {
                let callkit = matches!(
                    &want,
                    SessionWant::Call {
                        kind: CallKind::CallKit { .. },
                        ..
                    }
                );
                if callkit && (self.applied.is_some() || self.release_pending) {
                    self.deactivate(actions);
                }
                self.generation += 1;
                self.inflight = Some(self.generation);
                self.release_pending = false;
                if matches!(
                    &want,
                    SessionWant::Call {
                        kind: CallKind::Unmanaged,
                        ..
                    }
                ) {
                    self.own_mode = true;
                }
                self.applied = Some(want.clone());
                actions.push(AudioAction::Apply {
                    generation: self.generation,
                    want,
                    activate: !callkit,
                });
            }
        }
    }
    pub fn step(&mut self, input: AudioInput) -> Vec<AudioAction> {
        use AudioInput as I;
        if !matches!(input, I::Applied { .. }) {
            self.resuming = false;
        }
        let mut actions = Vec::new();
        let mut defer_reconcile = false;
        match input {
            I::VolumeReadStarted { id } => { self.volume_readers.insert(id); }
            I::VolumeReadEnded { id } => { self.volume_readers.remove(&id); }
            I::Audible { slot, on } => {
                if on {
                    self.audible.insert(slot);
                } else {
                    self.audible.remove(&slot);
                }
                self.system_event(SystemEvent::Audible(on), &mut actions);
            }
            I::Capture { id, kind } => {
                let refusal = if !self.permission {
                    Some(RecordingRefusal::PermissionDenied)
                } else if !self.calls.is_empty() || self.external() {
                    Some(RecordingRefusal::InCall)
                } else if self.system() == System::Interrupted {
                    Some(RecordingRefusal::Interrupted)
                } else if !self.captures.is_empty() || self.other_recording {
                    Some(RecordingRefusal::MicrophoneBusy)
                } else if kind == RecordingKind::Video && !self.flowing {
                    Some(RecordingRefusal::MicrophoneSilent)
                } else {
                    None
                };
                if let Some(reason) = refusal {
                    actions.push(AudioAction::Refuse { id, reason });
                } else {
                    self.captures.push((id, kind));
                    actions.push(AudioAction::Admit { id });
                    self.system_event(SystemEvent::Capture, &mut actions);
                    self.pause(PauseReason::Capture, &mut actions);
                }
            }
            I::CaptureEnded { id } => self.captures.retain(|(capture, _)| *capture != id),
            I::CallStarted { id, kind, speaker } => {
                self.calls.retain(|call| call.id != id);
                self.calls.push(Call {
                    id,
                    kind,
                    speaker,
                    state: CallState::Connecting,
                });
                actions.push(AudioAction::CallState {
                    id,
                    state: CallState::Connecting,
                });
                self.revoke(RecordingRefusal::InCall, &mut actions);
            }
            I::CallSpeaker { id, speaker } => {
                if let Some(call) = self.calls.iter_mut().find(|call| call.id == id) {
                    if call.kind != CallKind::Telecom {
                        call.speaker = speaker;
                    }
                }
            }
            I::CallEnded { id } => {
                if let Some(index) = self.calls.iter().position(|call| call.id == id) {
                    self.calls.remove(index);
                    actions.push(AudioAction::CallState {
                        id,
                        state: CallState::Ended,
                    });
                }
            }
            I::CallSystemActivated { id } => {
                if let Some(call) = self.calls.iter().find(|call| call.id == id) {
                    if matches!(call.kind, CallKind::CallKit { .. }) {
                        self.applied = Some(SessionWant::Call {
                            kind: call.kind.clone(),
                            speaker: call.speaker,
                        });
                    }
                    self.system_event(SystemEvent::CallSystemActivated, &mut actions);
                    self.call_state(id, CallState::Live, &mut actions);
                }
            }
            I::CallSystemDeactivated { id } => {
                self.call_state(id, CallState::Interrupted, &mut actions)
            }
            I::Applied { generation, result } => {
                if self.inflight == Some(generation) {
                    self.inflight = None;
                    match result {
                        AppliedResult::Ok => {
                            if matches!(
                                self.applied,
                                Some(SessionWant::Call {
                                    kind: CallKind::Unmanaged,
                                    ..
                                })
                            ) {
                                if let Some(call) = self.calls.last() {
                                    if call.kind == CallKind::Unmanaged
                                        && matches!(
                                            call.state,
                                            CallState::Connecting | CallState::Interrupted
                                        )
                                    {
                                        let id = call.id;
                                        self.call_state(id, CallState::Live, &mut actions);
                                    }
                                }
                            }
                        }
                        AppliedResult::Busy => {
                            self.release_pending = true;
                            defer_reconcile = true;
                        }
                        AppliedResult::Failed { .. } => {
                            self.applied = None;
                            self.system_event(SystemEvent::AppliedFailed, &mut actions);
                            self.revoke(RecordingRefusal::MicrophoneBusy, &mut actions);
                        }
                    }
                }
            }
            I::InterruptionBegan => {
                self.system_event(SystemEvent::InterruptionBegan, &mut actions);
                self.applied = None;
                self.revoke(RecordingRefusal::Interrupted, &mut actions);
                self.pause(PauseReason::Interrupted, &mut actions);
                for call in &mut self.calls {
                    if call.state == CallState::Live {
                        call.state = CallState::Interrupted;
                        actions.push(AudioAction::CallState {
                            id: call.id,
                            state: call.state,
                        });
                    }
                }
            }
            I::InterruptionEnded { should_resume } => {
                self.system_event(SystemEvent::InterruptionEnded(should_resume), &mut actions);
            }
            I::ResumptionRecommendation { should_resume } => {
                self.system_event(SystemEvent::ResumptionRecommendation(should_resume), &mut actions);
            }
            I::Focus { change } => match change {
                FocusChange::Loss => {
                    self.system_event(SystemEvent::Focus(change), &mut actions);
                    self.revoke(RecordingRefusal::Interrupted, &mut actions);
                    self.pause(PauseReason::FocusLost, &mut actions);
                }
                FocusChange::LossTransient => {
                    self.system_event(SystemEvent::Focus(change), &mut actions);
                    self.revoke(RecordingRefusal::Interrupted, &mut actions);
                    self.pause(PauseReason::Interrupted, &mut actions);
                }
                FocusChange::LossTransientDuck => {
                    self.system_event(SystemEvent::Focus(change), &mut actions);
                    actions.push(AudioAction::VolumeMultiplier { value: 0.2 });
                }
                FocusChange::Gain => self.system_event(SystemEvent::Focus(change), &mut actions),
            },
            I::FocusRequest { answer } => match answer {
                FocusAnswer::Granted => {}
                FocusAnswer::Delayed => {
                    self.system_event(SystemEvent::FocusRequest(answer), &mut actions);
                    self.pause(PauseReason::Interrupted, &mut actions);
                }
                FocusAnswer::Failed => {
                    self.applied = None;
                    self.system_event(SystemEvent::FocusRequest(answer), &mut actions);
                    self.pause(PauseReason::Interrupted, &mut actions);
                }
            },
            I::MediaReset => {
                self.applied = None;
                self.system_event(SystemEvent::MediaReset, &mut actions);
                self.revoke(RecordingRefusal::Interrupted, &mut actions);
                if !self.audible.is_empty() {
                    actions.push(AudioAction::PauseAll {
                        reason: PauseReason::MediaReset,
                    });
                }
                if self.paused.is_some() || !self.audible.is_empty() {
                    actions.push(AudioAction::ForgetPaused);
                }
                self.paused = None;
                self.audible.clear();
            }
            I::Route {
                reason,
                had_private_output,
            } => {
                if self.inflight.is_none()
                    && reason == RouteReason::OldDeviceUnavailable
                    && had_private_output
                {
                    self.pause(PauseReason::OutputLost, &mut actions);
                }
            }
            I::Noisy => self.pause(PauseReason::OutputLost, &mut actions),
            I::ObservedCalls { uuids } => {
                self.observed = uuids.into_iter().collect();
                if self.external() {
                    self.revoke(RecordingRefusal::InCall, &mut actions);
                }
            }
            I::PhoneCall { active } => {
                self.phone = active;
                if self.external() {
                    self.revoke(RecordingRefusal::InCall, &mut actions);
                }
            }
            I::CommunicationMode { active } => {
                self.comm_mode = active;
                if !active && self.calls.is_empty() {
                    self.own_mode = false;
                }
                if self.external() {
                    self.revoke(RecordingRefusal::InCall, &mut actions);
                }
            }
            I::Permission { granted } => {
                self.permission = granted;
                if !granted {
                    self.revoke(RecordingRefusal::PermissionDenied, &mut actions);
                }
            }
            I::OtherRecording { active } => self.other_recording = active,
            I::MicrophoneFlow { flowing } => {
                self.flowing = flowing;
                if !flowing {
                    self.captures.retain(|(id, kind)| {
                        if *kind == RecordingKind::Video {
                            actions.push(AudioAction::Revoke { id: *id, reason: RecordingRefusal::MicrophoneSilent });
                            false
                        } else {
                            true
                        }
                    });
                }
            }
            I::Silenced { id } => {
                if let Some(index) = self.captures.iter().position(|(capture, _)| *capture == id) {
                    self.captures.remove(index);
                    actions.push(AudioAction::Revoke {
                        id,
                        reason: RecordingRefusal::MicrophoneSilent,
                    });
                }
            }
            I::Foreground { on } => {
                self.foreground = on;
                self.system_event(SystemEvent::Foreground(on), &mut actions);
            }
        }
        if self.captures.is_empty() && self.paused == Some(PauseReason::Capture) {
            actions.push(AudioAction::ForgetPaused);
            self.paused = None;
        }
        if !defer_reconcile {
            self.reconcile(&mut actions);
        }
        let hold = !self.calls.is_empty() || self.external();
        if hold != self.hold {
            self.hold = hold;
            actions.push(AudioAction::CallHold { on: hold });
        }
        let availability = self.availability();
        if availability != self.last_availability {
            self.last_availability = availability;
            actions.push(AudioAction::Availability {
                value: availability,
            });
        }
        actions.sort_by_key(|action| match action {
            AudioAction::Revoke { .. } => 0,
            AudioAction::Refuse { .. } => 1,
            AudioAction::PauseAll { .. } => 2,
            AudioAction::ForgetPaused => 3,
            AudioAction::CallHold { .. } => 4,
            AudioAction::Deactivate { .. } => 5,
            AudioAction::Apply { .. } => 6,
            AudioAction::Admit { .. } => 7,
            AudioAction::ResumePaused => 8,
            AudioAction::VolumeMultiplier { .. } => 9,
            AudioAction::CallState { .. } => 10,
            AudioAction::Availability { .. } => 11,
        });
        actions
    }
}
