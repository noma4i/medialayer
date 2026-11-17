use statig::blocking::*;

use super::profile::{CameraLens, CaptureMode, CaptureProfile};
use super::refusal::CameraRefusal;
use super::rules::{flash_plan, recovery_step, record_verdict, zoom_target, FaultKind, RecordEnd, RecordVerdict, RecoveryAction};
use super::state::{CameraPhase, CaptureAction, CaptureEvent, CaptureRun, CaptureStep, TakeOutcome};

pub(super) struct Capture;

pub(super) enum CaptureRunEvent {
    Restore(CameraPhase),
    Input(CaptureEvent),
}

pub(super) struct CaptureRunInput {
    run: CaptureRun,
    profile: CaptureProfile,
    actions: Vec<CaptureAction>,
    cancel: bool,
}

pub(super) fn step(run: CaptureRun, event: CaptureEvent, profile: CaptureProfile) -> CaptureStep {
    let phase = run.state.phase;
    let mut context = CaptureRunInput { run, profile, actions: Vec::new(), cancel: false };
    let mut machine = Capture.state_machine();
    machine.handle_with_context(&CaptureRunEvent::Restore(phase), &mut context);
    machine.handle_with_context(&CaptureRunEvent::Input(event), &mut context);
    if context.cancel {
        machine.handle_with_context(&CaptureRunEvent::Input(CaptureEvent::Cancel), &mut context);
    }
    context.run.state.phase = match machine.state() {
        State::Idle {} => CameraPhase::Idle,
        State::Opening {} => CameraPhase::Opening,
        State::Ready {} => CameraPhase::Ready,
        State::Recording {} => CameraPhase::Recording,
        State::Stopping {} => CameraPhase::Stopping,
        State::Failed {} => CameraPhase::Failed,
        State::Closed {} => CameraPhase::Closed,
    };
    CaptureStep { run: context.run, actions: context.actions }
}

#[state_machine(initial = "State::idle()")]
impl Capture {
    #[state(superstate = "active")]
    fn idle(event: &CaptureRunEvent, context: &mut CaptureRunInput) -> Outcome<State> {
        match event {
            CaptureRunEvent::Input(CaptureEvent::DemandOn) => { context.demand_on(); Transition(State::opening()) }
            CaptureRunEvent::Input(CaptureEvent::DemandOff | CaptureEvent::Fault { .. } | CaptureEvent::Refuse { .. } | CaptureEvent::CameraClosed | CaptureEvent::Streaming { .. }) => Handled,
            CaptureRunEvent::Input(CaptureEvent::LayerClosed) => { context.layer_closed(false); Transition(State::closed()) }
            CaptureRunEvent::Input(CaptureEvent::SetLens { lens }) => {
                if context.run.state.shooting { context.defer_lens(*lens); } else { context.run.state.lens = *lens; }
                Handled
            }
            _ => Super,
        }
    }

    #[state(superstate = "active")]
    fn opening(event: &CaptureRunEvent, context: &mut CaptureRunInput) -> Outcome<State> {
        match event {
            CaptureRunEvent::Input(CaptureEvent::Opened { has_flash, zoom_min, zoom_max }) => {
                context.opened(*has_flash, *zoom_min, *zoom_max);
                if context.is_ready() { Transition(State::ready()) } else { Handled }
            }
            CaptureRunEvent::Input(CaptureEvent::Streaming { on }) => {
                context.run.streaming = *on;
                if context.is_ready() { Transition(State::ready()) } else { Handled }
            }
            CaptureRunEvent::Input(CaptureEvent::CameraClosed) => { context.run.open = false; Handled }
            CaptureRunEvent::Input(CaptureEvent::SetLens { lens }) => context.change_lens(*lens),
            _ => Super,
        }
    }

    #[state(superstate = "active")]
    fn ready(event: &CaptureRunEvent, context: &mut CaptureRunInput) -> Outcome<State> {
        match event {
            CaptureRunEvent::Input(CaptureEvent::CameraClosed) => { context.run.open = false; context.ready_or_opening() }
            CaptureRunEvent::Input(CaptureEvent::Streaming { on }) => {
                context.run.streaming = *on;
                if *on { Handled } else { context.ready_or_opening() }
            }
            CaptureRunEvent::Input(CaptureEvent::Photo) => { context.photo(); Handled }
            CaptureRunEvent::Input(CaptureEvent::Start) => { context.start(); Handled }
            CaptureRunEvent::Input(CaptureEvent::AudioFlow { flowing: true }) if context.run.started && !context.run.discard => {
                context.run.audio_flowing = Some(true);
                Transition(State::recording())
            }
            CaptureRunEvent::Input(CaptureEvent::Cancel) if context.recording_in_progress() => context.end_recording(true),
            CaptureRunEvent::Input(CaptureEvent::SetLens { lens }) => {
                if context.run.state.shooting { context.defer_lens(*lens); Handled }
                else if context.run.starting {
                    if *lens != context.run.state.lens { context.run.pending_lens = Some(*lens); context.end_recording(true) } else { Handled }
                } else { context.change_lens(*lens) }
            }
            CaptureRunEvent::Input(CaptureEvent::ZoomBy { factor }) => { context.zoom_by(*factor); Handled }
            _ => Super,
        }
    }

    #[state(superstate = "active")]
    fn recording(event: &CaptureRunEvent, context: &mut CaptureRunInput) -> Outcome<State> {
        match event {
            CaptureRunEvent::Input(CaptureEvent::Tick { recorded_ms }) => { context.run.state.recorded_ms = *recorded_ms; Handled }
            CaptureRunEvent::Input(CaptureEvent::Stop) => context.end_recording(false),
            CaptureRunEvent::Input(CaptureEvent::Cancel) => context.end_recording(true),
            CaptureRunEvent::Input(CaptureEvent::SetLens { lens }) if context.run.state.shooting => { context.defer_lens(*lens); Handled }
            CaptureRunEvent::Input(CaptureEvent::SetLens { lens }) if *lens != context.run.state.lens => { context.run.pending_lens = Some(*lens); context.end_recording(true) }
            CaptureRunEvent::Input(CaptureEvent::ZoomBy { factor }) => { context.zoom_by(*factor); Handled }
            _ => Super,
        }
    }

    #[state(superstate = "active")]
    fn stopping(event: &CaptureRunEvent, context: &mut CaptureRunInput) -> Outcome<State> {
        match event {
            CaptureRunEvent::Input(CaptureEvent::Cancel) => { context.run.discard = true; Handled }
            CaptureRunEvent::Input(CaptureEvent::RecordSettled { duration_ms }) if context.run.end.is_some() => Transition(context.settle(*duration_ms)),
            CaptureRunEvent::Input(CaptureEvent::SetLens { lens }) => { context.defer_lens(*lens); Handled }
            CaptureRunEvent::Input(CaptureEvent::ZoomBy { factor }) => { context.zoom_by(*factor); Handled }
            _ => Super,
        }
    }

    #[state(superstate = "active")]
    fn failed(event: &CaptureRunEvent, context: &mut CaptureRunInput) -> Outcome<State> {
        match event {
            CaptureRunEvent::Input(CaptureEvent::DemandOn) => { context.demand_on(); Transition(State::opening()) }
            CaptureRunEvent::Input(CaptureEvent::Fault { .. } | CaptureEvent::CameraClosed | CaptureEvent::Streaming { .. }) => Handled,
            CaptureRunEvent::Input(CaptureEvent::SetLens { lens }) if *lens != context.run.state.lens => { context.run.state.lens = *lens; context.demand_on(); Transition(State::opening()) }
            CaptureRunEvent::Input(CaptureEvent::SetLens { .. }) => Handled,
            _ => Super,
        }
    }

    #[state]
    fn closed() -> Outcome<State> { Handled }

    #[superstate]
    fn active(event: &CaptureRunEvent, context: &mut CaptureRunInput) -> Outcome<State> {
        match event {
            CaptureRunEvent::Restore(phase) => Transition(match phase {
                CameraPhase::Idle => State::idle(),
                CameraPhase::Opening => State::opening(),
                CameraPhase::Ready => State::ready(),
                CameraPhase::Recording => State::recording(),
                CameraPhase::Stopping => State::stopping(),
                CameraPhase::Failed => State::failed(),
                CameraPhase::Closed => State::closed(),
            }),
            CaptureRunEvent::Input(CaptureEvent::LayerClosed) => { context.layer_closed(true); Transition(State::closed()) }
            CaptureRunEvent::Input(CaptureEvent::DemandOff) => { context.reset_session(); context.actions.push(CaptureAction::Unbind); Transition(State::idle()) }
            CaptureRunEvent::Input(CaptureEvent::CameraClosed) => { context.run.open = false; Handled }
            CaptureRunEvent::Input(CaptureEvent::Streaming { on }) => { context.run.streaming = *on; Handled }
            CaptureRunEvent::Input(CaptureEvent::Fault { kind }) => Transition(context.fault(*kind)),
            CaptureRunEvent::Input(CaptureEvent::Refuse { reason }) => { context.fail(*reason); Transition(State::failed()) }
            CaptureRunEvent::Input(CaptureEvent::PhotoDone { ok }) if context.run.state.shooting => {
                context.run.state.shooting = false;
                context.run.state.fault = (!ok).then_some(CameraRefusal::PhotoFailed);
                context.run.state.refused_ms = None;
                if let Some(lens) = context.run.pending_lens.take() { context.rebind(lens); Transition(State::opening()) } else { Handled }
            }
            CaptureRunEvent::Input(CaptureEvent::RecordStarted) if context.run.starting => {
                context.run.starting = false;
                context.run.started = true;
                if !context.run.discard && (!context.profile.audio || context.run.audio_flowing == Some(true)) { Transition(State::recording()) } else { Handled }
            }
            CaptureRunEvent::Input(CaptureEvent::Revoked { reason }) if context.recording_in_progress() => {
                context.run.state.fault = Some(*reason);
                context.cancel = true;
                Handled
            }
            CaptureRunEvent::Input(CaptureEvent::AudioFlow { flowing }) => {
                context.run.audio_flowing = Some(*flowing);
                if !flowing && context.profile.audio && context.recording_in_progress() {
                    context.run.state.fault = Some(CameraRefusal::MicrophoneSilent);
                    context.cancel = true;
                    Handled
                } else { Handled }
            }
            CaptureRunEvent::Input(CaptureEvent::RecordFinished { end }) if context.recording_in_progress() => Transition(context.record_finished(*end)),
            CaptureRunEvent::Input(CaptureEvent::SetLens { lens }) if context.run.state.shooting => { context.defer_lens(*lens); Handled }
            CaptureRunEvent::Input(CaptureEvent::SetFlash { on }) => { context.set_flash(*on); Handled }
            _ => Handled,
        }
    }
}

impl CaptureRunInput {
    fn recording_in_progress(&self) -> bool { self.run.starting || self.run.started }

    fn reset_session(&mut self) {
        let state = &mut self.run.state;
        state.flash = false;
        state.zoom = 1.0;
        state.recorded_ms = 0;
        state.shooting = false;
        state.fault = None;
        state.refused_ms = None;
        self.run = CaptureRun { open: false, streaming: false, starting: false, started: false, discard: false, end: None, pending_lens: None, ..self.run };
    }

    fn demand_on(&mut self) {
        self.reset_session();
        self.run.recovery_spent = false;
        self.actions.push(CaptureAction::Bind { lens: self.run.state.lens });
    }

    fn layer_closed(&mut self, unbind: bool) {
        if self.recording_in_progress() {
            self.run.state.outcome = Some(TakeOutcome::Refused { reason: CameraRefusal::Interrupted });
            self.actions.push(CaptureAction::EndRecording { discard: true });
        }
        if unbind { self.actions.push(CaptureAction::Unbind); }
        self.reset_session();
    }

    fn opened(&mut self, has_flash: bool, zoom_min: f32, zoom_max: f32) {
        let state = &mut self.run.state;
        state.has_flash = has_flash;
        state.zoom_min = zoom_min;
        state.zoom_max = zoom_max.min(15.0).max(zoom_min);
        state.zoom = 1.0_f32.clamp(state.zoom_min, state.zoom_max);
        state.flash = flash_plan(state.flash, has_flash);
        self.run.open = true;
        if self.run.state.flash && self.profile.is_video() { self.actions.push(CaptureAction::SetTorch { on: true }); }
    }

    fn is_ready(&self) -> bool { self.run.open && self.run.streaming }

    fn ready_or_opening(&self) -> Outcome<State> {
        if !self.run.starting && !self.is_ready() { Transition(State::opening()) } else { Handled }
    }

    fn fault(&mut self, kind: FaultKind) -> State {
        match recovery_step(self.run.recovery_spent, kind) {
            RecoveryAction::Restart => {
                if self.recording_in_progress() { self.run.state.outcome = Some(TakeOutcome::Lost { reason: CameraRefusal::Interrupted }); }
                self.run.recovery_spent = true;
                let lens = self.run.state.lens;
                self.reset_session();
                self.actions.push(CaptureAction::Restart { lens });
                State::opening()
            }
            RecoveryAction::Fail => {
                self.fail(if kind == FaultKind::Reset { CameraRefusal::Busy } else { CameraRefusal::ConfigureFailed });
                State::failed()
            }
        }
    }

    fn fail(&mut self, reason: CameraRefusal) {
        if self.recording_in_progress() { self.run.state.outcome = Some(TakeOutcome::Refused { reason }); }
        self.reset_session();
        self.run.state.fault = Some(reason);
        self.actions.push(CaptureAction::Unbind);
    }

    fn photo(&mut self) {
        if self.profile.mode == CaptureMode::Photo && !self.run.state.shooting {
            self.run.state.shooting = true;
            self.actions.push(CaptureAction::TakePhoto);
        }
    }

    fn start(&mut self) {
        if !self.profile.is_video() || self.recording_in_progress() || self.run.state.shooting { return; }
        if self.profile.audio && self.run.audio_flowing == Some(false) { self.run.state.fault = Some(CameraRefusal::MicrophoneSilent); return; }
        self.run.starting = true;
        self.run.state.take = Some(self.run.state.take.unwrap_or(0).saturating_add(1));
        self.run.state.outcome = None;
        self.run.started = false;
        self.run.discard = false;
        self.run.end = None;
        self.run.state.recorded_ms = 0;
        self.run.state.fault = None;
        self.run.state.refused_ms = None;
        self.actions.push(CaptureAction::BeginRecording);
    }

    fn end_recording(&mut self, discard: bool) -> Outcome<State> {
        self.run.discard |= discard;
        self.actions.push(CaptureAction::EndRecording { discard });
        Transition(State::stopping())
    }

    fn record_finished(&mut self, end: RecordEnd) -> State {
        if self.profile.audio && self.run.audio_flowing != Some(true) && self.run.state.fault.is_none() {
            self.run.state.fault = Some(CameraRefusal::MicrophoneSilent);
            self.run.discard = true;
        }
        self.run.audio_flowing = None;
        self.run.starting = false;
        self.run.end = Some(end);
        if self.run.started && end != RecordEnd::Failed && !self.run.discard {
            self.actions.push(CaptureAction::ReadClip);
            State::stopping()
        } else { self.settle(None) }
    }

    fn settle(&mut self, duration_ms: Option<u64>) -> State {
        let end = self.run.end.unwrap_or(RecordEnd::Failed);
        let verdict = record_verdict(self.run.started, false, self.run.discard, end, duration_ms, self.profile.min_take_ms);
        let (action, fault) = match verdict {
            RecordVerdict::Deliver => (CaptureAction::Deliver, None),
            RecordVerdict::Skip => (CaptureAction::Discard, self.run.state.fault),
            RecordVerdict::TooShort => (CaptureAction::Discard, Some(CameraRefusal::TooShort)),
            RecordVerdict::CameraNotReady => (CaptureAction::Discard, Some(CameraRefusal::NotReady)),
            RecordVerdict::Failed => (CaptureAction::Discard, Some(CameraRefusal::RecordFailed)),
        };
        self.actions.push(action);
        self.run.state.fault = fault;
        self.run.state.outcome = match (action, fault, self.run.state.take) {
            (CaptureAction::Deliver, _, Some(take)) => Some(TakeOutcome::Captured { take }),
            (_, Some(reason), _) => Some(TakeOutcome::Refused { reason }),
            _ => Some(TakeOutcome::Refused { reason: CameraRefusal::Interrupted }),
        };
        self.run.state.refused_ms = duration_ms.filter(|_| fault == Some(CameraRefusal::TooShort));
        self.run.state.recorded_ms = 0;
        self.run.starting = false;
        self.run.started = false;
        self.run.discard = false;
        self.run.end = None;
        if let Some(lens) = self.run.pending_lens.take() { self.rebind(lens); State::opening() }
        else if self.is_ready() { State::ready() } else { State::opening() }
    }

    fn rebind(&mut self, lens: CameraLens) {
        self.run.state.lens = lens;
        self.run.open = false;
        self.run.streaming = false;
        if self.profile.mode == CaptureMode::Photo { self.run.state.flash = false; }
        self.actions.push(CaptureAction::Bind { lens });
    }

    fn defer_lens(&mut self, lens: CameraLens) { self.run.pending_lens = (lens != self.run.state.lens).then_some(lens); }

    fn change_lens(&mut self, lens: CameraLens) -> Outcome<State> {
        if self.run.state.shooting { self.defer_lens(lens); Handled }
        else if lens != self.run.state.lens { self.rebind(lens); Transition(State::opening()) }
        else { Handled }
    }

    fn set_flash(&mut self, on: bool) {
        let enabled = flash_plan(on, self.run.state.has_flash);
        if enabled == self.run.state.flash { return; }
        self.run.state.flash = enabled;
        if self.profile.is_video() { self.actions.push(CaptureAction::SetTorch { on: enabled }); }
    }

    fn zoom_by(&mut self, factor: f32) {
        let state = &self.run.state;
        let ratio = zoom_target(state.zoom, factor, state.zoom_min, state.zoom_max);
        if ratio != state.zoom { self.run.state.zoom = ratio; self.actions.push(CaptureAction::SetZoom { ratio }); }
    }
}
