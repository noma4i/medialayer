//! Focus transitions derive from AndroidX Media3 AudioFocusManager (Apache-2.0).
mod input;
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
struct Input {
    id: u64,
    kind: RecordingKind,
    generation: u64,
    ready: bool,
    /// The platform told this input is silent (no sound from a live source, or silenced before its first PCM); only its sound clears it.
    silent: bool,
    life: StateMachine<input::Life>,
}

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
    audible: BTreeSet<u64>,
    volume_readers: BTreeSet<u64>,
    captures: Vec<(u64, RecordingKind)>,
    taking: BTreeSet<u64>,
    calls: Vec<Call>,
    observed: BTreeSet<String>,
    phone: bool,
    comm_mode: bool,
    permission: MicrophonePermission,
    other_recording: bool,
    inputs: Vec<Input>,
    wake: bool,
    pending: BTreeSet<u64>,
    system: StateMachine<SystemMachine>,
    foreground: bool,
    session: StateMachine<session::Session>,
    generation: u64,
    paused: Option<Paused>,
    paused_audible: bool,
    resuming: bool,
    last_availability: MicrophoneAvailability,
    hold: bool,
}

fn feed_input(input: &mut Input, event: &input::InputEvent, actions: &mut Vec<AudioAction>, wake: &mut bool) {
    let mut context = input::InputContext { id: input.id, kind: input.kind, generation: &mut input.generation, actions, woke: wake };
    input.life.handle_with_context(event, &mut context);
}

impl AudioOwner {
    pub fn new(_focus: AudioFocus) -> Self {
        Self {
            audible: BTreeSet::new(),
            volume_readers: BTreeSet::new(),
            captures: Vec::new(),
            taking: BTreeSet::new(),
            calls: Vec::new(),
            observed: BTreeSet::new(),
            phone: false,
            comm_mode: false,
            permission: MicrophonePermission::Granted,
            other_recording: false,
            inputs: Vec::new(),
            wake: false,
            pending: BTreeSet::new(),
            system: SystemMachine.state_machine(),
            foreground: true,
            session: session::Session.state_machine(),
            generation: 0,
            paused: None,
            paused_audible: false,
            resuming: false,
            last_availability: MicrophoneAvailability::Available,
            hold: false,
        }
    }

    fn system(&self) -> System {
        current(&self.system)
    }
    fn system_event(&mut self, event: SystemEvent, actions: &mut Vec<AudioAction>) {
        let before = self.system();
        let mut exit = Exit::default();
        self.system.handle_with_context(&event, &mut exit);
        let left_blocked = matches!(before, System::Interrupted | System::Transient | System::WaitForeground)
            && !matches!(self.system(), System::Interrupted | System::Transient | System::WaitForeground);
        if let Some(resume) = exit.resume.or(left_blocked.then_some(false)) {
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
        if self.permission == MicrophonePermission::NotAsked {
            MicrophoneAvailability::PermissionNotAsked
        } else if self.permission == MicrophonePermission::Denied {
            MicrophoneAvailability::PermissionDenied
        } else if !self.calls.is_empty() {
            MicrophoneAvailability::OwnCall
        } else if self.external() {
            MicrophoneAvailability::ExternalCall
        } else if self.other_recording || self.system_blocked() {
            MicrophoneAvailability::OtherApp
        } else if self.inputs.iter().any(|input| input.silent && !self.yields(input.kind)) {
            MicrophoneAvailability::NoSound
        } else {
            MicrophoneAvailability::Available
        }
    }
    fn system_blocked(&self) -> bool {
        matches!(self.system(), System::Interrupted | System::FocusLost | System::Transient | System::WaitForeground) || session::blocked(&self.session)
    }
    /// One AudioRecord per app: an input of the camera preview yields the microphone to a voice capture (Android silences the second AudioRecord of an app).
    fn yields(&self, kind: RecordingKind) -> bool {
        kind == RecordingKind::Video && (self.captures.iter().any(|(_, existing)| *existing == RecordingKind::Voice) || self.inputs.iter().any(|input| input.kind == RecordingKind::Voice))
    }
    fn video_taking(&self) -> bool {
        self.captures.iter().any(|(id, kind)| *kind == RecordingKind::Video && self.taking.contains(id))
    }
    fn feed(&mut self, id: u64, event: input::InputEvent, actions: &mut Vec<AudioAction>) {
        if let Some(input) = self.inputs.iter_mut().find(|input| input.id == id) { feed_input(input, &event, actions, &mut self.wake); }
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
            if !self.captures.is_empty() || !self.inputs.is_empty() {
                let video = self.captures.iter().any(|(_, kind)| *kind == RecordingKind::Video) || self.inputs.iter().any(|input| input.kind == RecordingKind::Video);
                return Some(SessionWant::Recording { kind: if video { RecordingKind::Video } else { RecordingKind::Voice } });
            }
        }
        if (!self.audible.is_empty() || (self.paused.is_some() && self.paused_audible) || self.resuming)
            && matches!(
                self.system(),
                System::Free | System::Ducked | System::Transient
            )
        {
            return Some(SessionWant::Playback { focus: AudioFocus::Exclusive });
        }
        if !self.volume_readers.is_empty()
            && matches!(self.system(), System::Free | System::Ducked | System::Transient)
        {
            return Some(SessionWant::Playback { focus: AudioFocus::Mix });
        }
        None
    }
    fn revoke(&mut self, reason: RecordingRefusal, actions: &mut Vec<AudioAction>) {
        self.taking.clear();
        for (id, _) in self.captures.drain(..) {
            actions.push(if self.pending.remove(&id) { AudioAction::Refuse { id, reason } } else { AudioAction::Revoke { id, reason } });
        }
    }
    fn pause(&mut self, reason: PauseReason, actions: &mut Vec<AudioAction>) {
        actions.push(AudioAction::PauseAll { reason });
        if matches!(reason, PauseReason::Capture | PauseReason::Interrupted) && self.paused != Some(Paused::Capture) {
            self.paused_audible |= !self.audible.is_empty();
            self.paused = Some(if reason == PauseReason::Capture { Paused::Capture } else { Paused::Interrupted { resume: false } });
        }
        if !matches!(reason, PauseReason::Capture | PauseReason::Interrupted) && self.paused != Some(Paused::Capture) && (self.paused.is_some() || !self.audible.is_empty()) {
            actions.push(AudioAction::ForgetPaused);
            self.paused = None;
            self.paused_audible = false;
        }
    }
    fn finish_interruption(&mut self, resume: bool, actions: &mut Vec<AudioAction>) {
        if matches!(self.paused, Some(Paused::Interrupted { .. })) {
            if resume {
                self.paused = Some(Paused::Interrupted { resume: true });
            } else {
                actions.push(AudioAction::ForgetPaused);
                self.paused = None;
                self.paused_audible = false;
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
    fn interrupt_calls(&mut self, actions: &mut Vec<AudioAction>) {
        for call in &mut self.calls {
            if call.state == CallState::Live {
                call.state = CallState::Interrupted;
                actions.push(AudioAction::CallState { id: call.id, state: call.state });
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
    #[cfg(test)]
    fn paused_audible(&self) -> bool { self.paused.is_some() && self.paused_audible }
    pub fn step(&mut self, input: AudioInput) -> Vec<AudioAction> {
        use AudioInput as I;
        if let I::Failure { failure } = input { return vec![AudioAction::Failure { failure }]; }
        if matches!(&input, I::FocusRequest { generation, .. } | I::Applied { generation, .. } if session::inflight(&self.session) != Some(*generation)) {
            return Vec::new();
        }
        if matches!(&input, I::FocusRequest { .. }) && !session::asking(&self.session) { return Vec::new(); }
        let next = !matches!(input, I::Applied { .. } | I::FocusRequest { .. });
        let call_applied = matches!(&input, I::Applied { result: AppliedResult::Ok, .. } | I::FocusRequest { answer: FocusAnswer::Granted, .. });
        let own_before = session::own_mode(&self.session);
        let mut fresh = matches!(&input, I::InterruptionEnded { .. } | I::ResumptionRecommendation { .. } | I::Focus { change: FocusChange::Gain } | I::Foreground { on: true } | I::MediaReset);
        let mut audible_edge = false;
        let mut actions = Vec::new();
        if !matches!(input, I::Applied { .. }) {
            self.resuming = false;
        }
        if next {
            self.session_event(session::SessionEvent::Other, &mut actions);
        }
        match input {
            I::Failure { failure } => actions.push(AudioAction::Failure { failure }),
            I::VolumeReadStarted { id } => { fresh = self.volume_readers.insert(id); }
            I::VolumeReadEnded { id } => { self.volume_readers.remove(&id); }
            I::Audible { slot, on } => {
                if on {
                    fresh = self.audible.insert(slot);
                    audible_edge = fresh;
                } else {
                    self.audible.remove(&slot);
                }
                self.system_event(SystemEvent::Audible(on), &mut actions);
            }
            I::Capture { id, kind } => {
                let refusal = if self.permission != MicrophonePermission::Granted {
                    Some(RecordingRefusal::PermissionDenied)
                } else if !self.calls.is_empty() || self.external() {
                    Some(RecordingRefusal::InCall)
                } else if self.system() == System::Interrupted {
                    Some(RecordingRefusal::Interrupted)
                } else if self.captures.iter().any(|(_, existing)| *existing == kind) || (kind == RecordingKind::Voice && self.video_taking()) {
                    // A recording of another app never blocks a start: the one who opens a recording takes the microphone from the system (owner rule 2026-11-19).
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
                self.taking.remove(&id);
            }
            I::CaptureTake { id, taking } => {
                let video = self.captures.iter().any(|(capture, kind)| *capture == id && *kind == RecordingKind::Video);
                if !taking || !video {
                    self.taking.remove(&id);
                } else if self.yields(RecordingKind::Video) {
                    // The voice holds the microphone: the take of the camera is the later one and is refused busy.
                    self.captures.retain(|(capture, _)| *capture != id);
                    let reason = RecordingRefusal::MicrophoneBusy;
                    actions.push(if self.pending.remove(&id) { AudioAction::Refuse { id, reason } } else { AudioAction::Revoke { id, reason } });
                } else {
                    self.taking.insert(id);
                }
            }
            I::CallStarted { id, kind, speaker } => {
                fresh = !self.calls.iter().any(|call| call.id == id);
                if fresh && self.system() == System::FocusLost { self.system_event(SystemEvent::NewDemand, &mut actions); }
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
                let newest = self.calls.last().is_some_and(|call| call.id == id);
                if let Some(call) = self.calls.iter_mut().find(|call| call.id == id) {
                    let changed = call.speaker != speaker;
                    call.speaker = speaker;
                    if newest && changed && call.kind != CallKind::Unmanaged {
                        self.session_event(session::SessionEvent::ManagedSpeaker(speaker), &mut actions);
                        actions.push(AudioAction::Route { speaker });
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
                    self.interrupt_calls(&mut actions);
                    self.system_event(SystemEvent::AppliedFailed, &mut actions);
                    self.revoke(RecordingRefusal::MicrophoneBusy, &mut actions);
                    if matches!(self.paused, Some(Paused::Interrupted { resume: true })) {
                        actions.push(AudioAction::ForgetPaused);
                        self.paused = None;
                        self.paused_audible = false;
                    }
                    if !self.audible.is_empty() { self.pause(PauseReason::Interrupted, &mut actions); }
                }
            }
            I::InterruptionBegan => {
                self.system_event(SystemEvent::InterruptionBegan, &mut actions);
                self.session_event(session::SessionEvent::Reset, &mut actions);
                self.revoke(RecordingRefusal::Interrupted, &mut actions);
                self.pause(PauseReason::Interrupted, &mut actions);
                self.interrupt_calls(&mut actions);
            }
            I::InterruptionEnded { should_resume } => {
                self.system_event(SystemEvent::InterruptionEnded(should_resume), &mut actions);
            }
            I::ResumptionRecommendation { should_resume } => {
                self.system_event(SystemEvent::ResumptionRecommendation(should_resume), &mut actions);
            }
            I::Focus { change } => match change {
                FocusChange::Loss => {
                    self.interrupt_calls(&mut actions);
                    self.system_event(SystemEvent::Focus(change), &mut actions);
                    self.revoke(RecordingRefusal::Interrupted, &mut actions);
                    self.pause(PauseReason::FocusLost, &mut actions);
                }
                FocusChange::LossTransient => {
                    self.interrupt_calls(&mut actions);
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
                self.interrupt_calls(&mut actions);
                self.session_event(session::SessionEvent::Reset, &mut actions);
                self.system_event(SystemEvent::MediaReset, &mut actions);
                self.revoke(RecordingRefusal::Interrupted, &mut actions);
                actions.push(AudioAction::PauseAll { reason: PauseReason::MediaReset });
                if self.paused.is_some() || !self.audible.is_empty() {
                    actions.push(AudioAction::ForgetPaused);
                }
                self.paused = None;
                self.paused_audible = false;
                self.audible.clear();
            }
            I::Route {
                reason,
                lost_private_output,
            } => {
                self.wake |= matches!(reason, RouteReason::NewDeviceAvailable | RouteReason::OldDeviceUnavailable);
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
                if active { self.interrupt_calls(&mut actions); }
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
            I::Permission { state } => {
                self.permission = state;
                if state != MicrophonePermission::Granted {
                    self.revoke(RecordingRefusal::PermissionDenied, &mut actions);
                }
            }
            I::OtherRecording { active } => self.other_recording = active,
            I::Silenced { id } => {
                if let Some(index) = self.captures.iter().position(|(capture, _)| *capture == id) {
                    self.captures.remove(index);
                    self.taking.remove(&id);
                    let reason = RecordingRefusal::MicrophoneSilent;
                    actions.push(if self.pending.remove(&id) { AudioAction::Refuse { id, reason } } else { AudioAction::Revoke { id, reason } });
                }
            }
            I::InputOpened { id, kind } => {
                if !self.inputs.iter().any(|input| input.id == id) {
                    self.inputs.push(Input { id, kind, generation: 0, ready: false, silent: false, life: input::Life.state_machine() });
                    fresh = true;
                }
            }
            I::InputClosed { id } => {
                self.feed(id, input::InputEvent::Block, &mut actions);
                self.inputs.retain(|input| input.id != id);
            }
            I::NoSound { id, generation, on } => {
                // NoSound is only the platform's word of silence for the current generation, never "not heard yet".
                if let Some(input) = self.inputs.iter_mut().find(|input| input.id == id && input.generation == generation) { input.silent = on; }
                self.feed(id, input::InputEvent::Sound { generation, heard: !on }, &mut actions)
            }
            I::RestoreFailed { id, generation } => {
                // A start the platform refused is its word that this input gives no sound.
                if let Some(input) = self.inputs.iter_mut().find(|input| input.id == id && input.generation == generation) { input.silent = true; }
                self.feed(id, input::InputEvent::Failed { generation }, &mut actions)
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
            if let SessionWant::Recording { kind: held_kind } = held {
                for (id, kind) in &self.captures {
                    if (held_kind == RecordingKind::Video || held_kind == *kind) && self.pending.remove(id) { actions.push(AudioAction::Admit { id: *id }); }
                }
            }
            if call_applied && matches!(held, SessionWant::Call { kind: CallKind::Unmanaged, .. }) {
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
            self.paused_audible = false;
        }
        if fresh && (session::blocked(&self.session) || self.system() == System::WaitForeground) {
            self.system_event(SystemEvent::NewDemand, &mut actions);
        }
        let want = self.want();
        audible_edge &= matches!(want, Some(SessionWant::Playback { .. }));
        self.session_event(session::SessionEvent::Reconcile { want, may_ask: self.may_ask(), fresh, next, audible_edge }, &mut actions);
        if own_before && !session::own_mode(&self.session) { self.comm_mode = false; }
        if self.paused == Some(Paused::Interrupted { resume: true }) && ((session::held(&self.session).is_some() && session::held(&self.session) == self.want()) || (!self.paused_audible && self.want().is_none() && self.system() == System::Free)) {
            actions.push(AudioAction::ResumePaused);
            self.paused = None;
            self.resuming = self.paused_audible;
            self.paused_audible = false;
        }
        let hold = !self.calls.is_empty() || self.external();
        if hold != self.hold {
            self.hold = hold;
            actions.push(AudioAction::CallHold { on: hold });
        }
        let open = self.permission == MicrophonePermission::Granted && self.foreground && !hold && !self.system_blocked();
        let held = match session::held(&self.session) { Some(SessionWant::Recording { kind }) => Some(kind), _ => None };
        let woken = self.wake;
        let voice = self.yields(RecordingKind::Video);
        for input in &mut self.inputs {
            let ready = open && held.is_some_and(|held| held == RecordingKind::Video || held == input.kind) && !(input.kind == RecordingKind::Video && voice);
            let edge = ready && !input.ready;
            input.ready = ready;
            if !ready { input.silent = false; feed_input(input, &input::InputEvent::Block, &mut actions, &mut self.wake); }
            else if edge || woken { feed_input(input, &input::InputEvent::Wake, &mut actions, &mut self.wake); }
        }
        self.wake = false;
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
            AudioAction::AwaitPaused { .. } => 5,
            AudioAction::Deactivate { .. } => 5,
            AudioAction::Apply { .. } => 6,
            AudioAction::Admit { .. } => 7,
            AudioAction::ResumePaused => 8,
            AudioAction::VolumeMultiplier { .. } => 9,
            AudioAction::CallState { .. } => 10,
            AudioAction::Availability { .. } | AudioAction::RestoreInput { .. } => 11,
            AudioAction::StopInput { .. } => 2,
            AudioAction::Route { .. } | AudioAction::Failure { .. } => 12,
        });
        actions
    }
}
