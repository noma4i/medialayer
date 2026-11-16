//! Focus transitions derive from AndroidX Media3 AudioFocusManager (Apache-2.0).
mod session;
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Paused { Capture, Interrupted { resume: bool } }

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
    permission: bool,
    other_recording: bool,
    pending: BTreeSet<u64>,
    system: StateMachine<SystemMachine>,
    foreground: bool,
    session: StateMachine<session::Session>,
    generation: u64,
    paused: Option<Paused>,
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
            permission: true,
            other_recording: false,
            pending: BTreeSet::new(),
            system: SystemMachine.state_machine(),
            foreground: true,
            session: session::Session.state_machine(),
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
            || (self.comm_mode && !session::own_mode(&self.session))
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
            if !self.captures.is_empty() {
                let kind = if self.captures.iter().any(|(_, kind)| *kind == RecordingKind::Video) { RecordingKind::Video } else { RecordingKind::Voice };
                return Some(SessionWant::Recording { kind });
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
            actions.push(if self.pending.remove(&id) { AudioAction::Refuse { id, reason } } else { AudioAction::Revoke { id, reason } });
        }
    }
    fn pause(&mut self, reason: PauseReason, actions: &mut Vec<AudioAction>) {
        if !self.audible.is_empty() || reason == PauseReason::Capture {
            actions.push(AudioAction::PauseAll { reason });
            if matches!(reason, PauseReason::Capture | PauseReason::Interrupted) {
                self.paused = Some(if reason == PauseReason::Capture { Paused::Capture } else { Paused::Interrupted { resume: false } });
            }
        }
        if !matches!(reason, PauseReason::Capture | PauseReason::Interrupted) && self.paused != Some(Paused::Capture) && (self.paused.is_some() || !self.audible.is_empty()) {
            actions.push(AudioAction::ForgetPaused);
            self.paused = None;
        }
    }
    fn finish_interruption(&mut self, resume: bool, actions: &mut Vec<AudioAction>) {
        if matches!(self.paused, Some(Paused::Interrupted { .. })) {
            if resume {
                self.paused = Some(Paused::Interrupted { resume: true });
            } else {
                actions.push(AudioAction::ForgetPaused);
                self.paused = None;
            }
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
    fn session_event(&mut self, event: session::SessionEvent, actions: &mut Vec<AudioAction>) {
        let mut context = session::SessionContext { generation: &mut self.generation, actions };
        self.session.handle_with_context(&event, &mut context);
    }
    fn may_ask(&self) -> bool { matches!(self.system(), System::Free | System::Ducked) }
    #[cfg(test)]
    fn inflight(&self) -> Option<u64> { session::inflight(&self.session) }
    #[cfg(test)]
    fn applied(&self) -> Option<SessionWant> { session::applied(&self.session) }
    #[cfg(test)]
    fn request_blocked(&self) -> bool { session::blocked(&self.session) || !self.may_ask() }
    #[cfg(test)]
    fn release_pending(&self) -> bool { session::lingering(&self.session) }
    #[cfg(test)]
    fn pending(&self) -> &BTreeSet<u64> { &self.pending }
    #[cfg(test)]
    fn captures(&self) -> &[(u64, RecordingKind)] { &self.captures }
    #[cfg(test)]
    fn paused(&self) -> Option<PauseReason> {
        self.paused.map(|value| match value { Paused::Capture => PauseReason::Capture, Paused::Interrupted { .. } => PauseReason::Interrupted })
    }
    pub fn step(&mut self, input: AudioInput) -> Vec<AudioAction> {
        use AudioInput as I;
        if matches!(&input, I::FocusRequest { generation, .. } | I::Applied { generation, .. } if session::inflight(&self.session) != Some(*generation)) {
            return Vec::new();
        }
        if matches!(&input, I::FocusRequest { .. }) && !session::asking(&self.session) { return Vec::new(); }
        let next = !matches!(input, I::Applied { .. } | I::FocusRequest { .. });
        let own_before = session::own_mode(&self.session);
        let mut fresh = matches!(&input, I::InterruptionEnded { .. } | I::ResumptionRecommendation { .. } | I::Focus { change: FocusChange::Gain } | I::Foreground { on: true } | I::MediaReset);
        let mut actions = Vec::new();
        if !matches!(input, I::Applied { .. }) {
            self.resuming = false;
        }
        if next {
            self.session_event(session::SessionEvent::Other, &mut actions);
        }
        match input {
            I::VolumeReadStarted { id } => { fresh = self.volume_readers.insert(id); }
            I::VolumeReadEnded { id } => { self.volume_readers.remove(&id); }
            I::Audible { slot, on } => {
                if on {
                    fresh = self.audible.insert(slot);
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
                } else if self.captures.iter().any(|(_, existing)| *existing == kind) || self.other_recording {
                    Some(RecordingRefusal::MicrophoneBusy)
                } else {
                    None
                };
                if let Some(reason) = refusal {
                    actions.push(AudioAction::Refuse { id, reason });
                } else {
                    fresh = true;
                    let first = self.captures.is_empty();
                    self.captures.push((id, kind));
                    if matches!(session::held(&self.session), Some(SessionWant::Recording { kind: RecordingKind::Video })) || session::held(&self.session) == Some(SessionWant::Recording { kind }) {
                        actions.push(AudioAction::Admit { id });
                    } else {
                        self.pending.insert(id);
                    }
                    self.system_event(SystemEvent::Capture, &mut actions);
                    if first { self.pause(PauseReason::Capture, &mut actions); }
                }
            }
            I::CaptureEnded { id } => {
                self.captures.retain(|(capture, _)| *capture != id);
                self.pending.remove(&id);
            }
            I::CallStarted { id, kind, speaker } => {
                fresh = !self.calls.iter().any(|call| call.id == id);
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
                    fresh = true;
                    if matches!(call.kind, CallKind::CallKit { .. }) {
                        let want = SessionWant::Call { kind: call.kind.clone(), speaker: call.speaker };
                        self.session_event(session::SessionEvent::CallActivated(want), &mut actions);
                    }
                    self.system_event(SystemEvent::CallSystemActivated, &mut actions);
                    self.call_state(id, CallState::Live, &mut actions);
                }
            }
            I::CallSystemDeactivated { id } => {
                self.call_state(id, CallState::Interrupted, &mut actions)
            }
            I::Applied { generation, result } => {
                let asking = session::applied(&self.session).is_some();
                self.session_event(session::SessionEvent::Applied { generation, result: result.clone() }, &mut actions);
                if asking && matches!(result, AppliedResult::Failed { .. }) {
                    self.system_event(SystemEvent::AppliedFailed, &mut actions);
                    self.revoke(RecordingRefusal::MicrophoneBusy, &mut actions);
                    if matches!(self.paused, Some(Paused::Interrupted { resume: true })) {
                        actions.push(AudioAction::ForgetPaused);
                        self.paused = None;
                    }
                    if !self.audible.is_empty() { self.pause(PauseReason::Interrupted, &mut actions); }
                }
            }
            I::InterruptionBegan => {
                self.system_event(SystemEvent::InterruptionBegan, &mut actions);
                self.session_event(session::SessionEvent::Reset, &mut actions);
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
                FocusChange::Gain => {
                    self.session_event(session::SessionEvent::Gain, &mut actions);
                    self.system_event(SystemEvent::Focus(change), &mut actions);
                }
            },
            I::FocusRequest { generation, answer } => {
                self.session_event(session::SessionEvent::Focus { generation, answer }, &mut actions);
                if matches!(answer, FocusAnswer::Delayed | FocusAnswer::Failed) {
                    self.revoke(RecordingRefusal::MicrophoneBusy, &mut actions);
                    self.system_event(SystemEvent::FocusRequest(answer), &mut actions);
                    self.pause(PauseReason::Interrupted, &mut actions);
                }
            }
            I::MediaReset => {
                self.session_event(session::SessionEvent::Reset, &mut actions);
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
                lost_private_output,
            } => {
                if reason == RouteReason::OldDeviceUnavailable
                    && lost_private_output
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
            I::Silenced { id } => {
                if let Some(index) = self.captures.iter().position(|(capture, _)| *capture == id) {
                    self.captures.remove(index);
                    let reason = RecordingRefusal::MicrophoneSilent;
                    actions.push(if self.pending.remove(&id) { AudioAction::Refuse { id, reason } } else { AudioAction::Revoke { id, reason } });
                }
            }
            I::Foreground { on } => {
                self.foreground = on;
                self.system_event(SystemEvent::Foreground(on), &mut actions);
            }
        }
        if own_before && !session::own_mode(&self.session) {
            self.comm_mode = false;
        }
        if let Some(held) = session::held(&self.session) {
            if matches!(held, SessionWant::Recording { .. }) {
                for (id, _) in &self.captures {
                    if self.pending.remove(id) { actions.push(AudioAction::Admit { id: *id }); }
                }
            }
            if matches!(held, SessionWant::Call { kind: CallKind::Unmanaged, .. }) {
                if let Some(call) = self.calls.last() {
                    if call.kind == CallKind::Unmanaged && matches!(call.state, CallState::Connecting | CallState::Interrupted) {
                        self.call_state(call.id, CallState::Live, &mut actions);
                    }
                }
            }
        }
        if self.captures.is_empty() && self.paused == Some(Paused::Capture) {
            actions.push(AudioAction::ForgetPaused);
            self.paused = None;
        }
        if fresh && (session::blocked(&self.session) || self.system() == System::WaitForeground) {
            self.system_event(SystemEvent::NewDemand, &mut actions);
        }
        let want = self.want();
        self.session_event(session::SessionEvent::Reconcile { want, may_ask: self.may_ask(), fresh, next }, &mut actions);
        if own_before && !session::own_mode(&self.session) { self.comm_mode = false; }
        if self.paused == Some(Paused::Interrupted { resume: true }) && session::held(&self.session).is_some() && session::held(&self.session) == self.want() {
            actions.push(AudioAction::ResumePaused);
            self.paused = None;
            self.resuming = true;
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
