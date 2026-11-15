use super::profile::{CameraLens, CaptureMode, CaptureProfile};
use super::refusal::CameraRefusal;
use super::rules::{flash_plan, recovery_step, record_verdict, zoom_target, FaultKind, RecordEnd, RecordVerdict, RecoveryAction};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CameraPhase {
    Idle,
    Opening,
    Ready,
    Recording,
    Stopping,
    Failed,
}

/// What the app sees of a camera. `shooting` is true while a photo is taken, `recorded_ms` grows only in `Recording`, `refused_ms` is the length of the recording a `TooShort` fault refused.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CameraState {
    pub phase: CameraPhase,
    pub lens: CameraLens,
    pub flash: bool,
    pub has_flash: bool,
    pub zoom: f32,
    pub zoom_min: f32,
    pub zoom_max: f32,
    pub recorded_ms: u64,
    pub shooting: bool,
    pub fault: Option<CameraRefusal>,
    pub refused_ms: Option<u64>,
}

/// The state of a camera and what the machine remembers between steps. The platform keeps one run per camera and replaces it with the step result.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CaptureRun {
    pub state: CameraState,
    pub open: bool,
    pub streaming: bool,
    pub starting: bool,
    pub started: bool,
    pub discard: bool,
    pub end: Option<RecordEnd>,
    pub recovery_spent: bool,
    pub pending_lens: Option<CameraLens>,
}

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Enum)]
pub enum CaptureEvent {
    DemandOn,
    DemandOff,
    Opened { has_flash: bool, zoom_min: f32, zoom_max: f32 },
    CameraClosed,
    Streaming { on: bool },
    Fault { kind: FaultKind },
    Refuse { reason: CameraRefusal },
    Photo,
    PhotoDone { ok: bool },
    Start,
    RecordStarted,
    Tick { recorded_ms: u64 },
    Stop,
    Cancel,
    /// A call began: a recording that runs is thrown away and ends with `InCall`.
    CallStarted,
    RecordFinished { end: RecordEnd },
    RecordSettled { duration_ms: Option<u64> },
    SetLens { lens: CameraLens },
    SetFlash { on: bool },
    ZoomBy { factor: f32 },
}

/// What the platform does after a step. `Unbind` and `Restart` also stop a running recording and delete its file.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Enum)]
pub enum CaptureAction {
    Bind { lens: CameraLens },
    Unbind,
    Restart { lens: CameraLens },
    TakePhoto,
    BeginRecording,
    EndRecording { discard: bool },
    ReadClip,
    Deliver,
    Discard,
    SetTorch { on: bool },
    SetZoom { ratio: f32 },
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CaptureStep {
    pub run: CaptureRun,
    pub actions: Vec<CaptureAction>,
}

/// A camera at rest on `lens`.
#[uniffi::export]
pub fn capture_run(lens: CameraLens) -> CaptureRun {
    CaptureRun {
        state: CameraState { phase: CameraPhase::Idle, lens, flash: false, has_flash: false, zoom: 1.0, zoom_min: 1.0, zoom_max: 1.0, recorded_ms: 0, shooting: false, fault: None, refused_ms: None },
        open: false,
        streaming: false,
        starting: false,
        started: false,
        discard: false,
        end: None,
        recovery_spent: false,
        pending_lens: None,
    }
}

/// The one machine of a camera: the next run and the actions for a platform event or an app command. A command that does not fit the phase changes nothing.
#[uniffi::export]
pub fn capture_step(run: CaptureRun, event: CaptureEvent, profile: CaptureProfile) -> CaptureStep {
    let mut machine = Machine { run, profile, actions: Vec::new() };
    machine.apply(event);
    CaptureStep { run: machine.run, actions: machine.actions }
}

struct Machine {
    run: CaptureRun,
    profile: CaptureProfile,
    actions: Vec<CaptureAction>,
}

impl Machine {
    fn phase(&self) -> CameraPhase {
        self.run.state.phase
    }

    fn recording_in_progress(&self) -> bool {
        self.run.starting || self.run.started
    }

    fn apply(&mut self, event: CaptureEvent) {
        let phase = self.phase();
        match event {
            CaptureEvent::DemandOn => self.demand_on(),
            CaptureEvent::DemandOff => self.demand_off(),
            CaptureEvent::Opened { has_flash, zoom_min, zoom_max } if phase == CameraPhase::Opening => self.opened(has_flash, zoom_min, zoom_max),
            CaptureEvent::CameraClosed if !matches!(phase, CameraPhase::Idle | CameraPhase::Failed) => {
                self.run.open = false;
                self.leave_ready();
            }
            CaptureEvent::Streaming { on } if !matches!(phase, CameraPhase::Idle | CameraPhase::Failed) => {
                self.run.streaming = on;
                self.enter_ready();
                if !on {
                    self.leave_ready();
                }
            }
            CaptureEvent::Fault { kind } => self.fault(kind),
            CaptureEvent::Refuse { reason } if phase != CameraPhase::Idle => self.fail(reason),
            CaptureEvent::Photo => self.photo(),
            CaptureEvent::PhotoDone { ok } if self.run.state.shooting => {
                self.run.state.shooting = false;
                self.run.state.fault = (!ok).then_some(CameraRefusal::PhotoFailed);
                self.run.state.refused_ms = None;
                if let Some(lens) = self.run.pending_lens.take() {
                    self.rebind(lens);
                }
            }
            CaptureEvent::Start => self.start(),
            CaptureEvent::RecordStarted if self.run.starting => {
                self.run.starting = false;
                self.run.started = true;
                if !self.run.discard {
                    self.run.state.phase = CameraPhase::Recording;
                }
            }
            CaptureEvent::Tick { recorded_ms } if phase == CameraPhase::Recording => self.run.state.recorded_ms = recorded_ms,
            CaptureEvent::Stop if phase == CameraPhase::Recording => self.end_recording(false),
            CaptureEvent::Cancel => self.cancel(),
            CaptureEvent::CallStarted if self.recording_in_progress() => {
                self.run.state.fault = Some(CameraRefusal::InCall);
                self.cancel();
            }
            CaptureEvent::RecordFinished { end } if self.recording_in_progress() => self.record_finished(end),
            CaptureEvent::RecordSettled { duration_ms } if phase == CameraPhase::Stopping && self.run.end.is_some() => self.settle(duration_ms),
            CaptureEvent::SetLens { lens } => self.set_lens(lens),
            CaptureEvent::SetFlash { on } => self.set_flash(on),
            CaptureEvent::ZoomBy { factor } => self.zoom_by(factor),
            _ => {}
        }
    }

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
        if !matches!(self.phase(), CameraPhase::Idle | CameraPhase::Failed) {
            return;
        }
        self.reset_session();
        self.run.recovery_spent = false;
        self.run.state.phase = CameraPhase::Opening;
        self.actions.push(CaptureAction::Bind { lens: self.run.state.lens });
    }

    fn demand_off(&mut self) {
        if self.phase() == CameraPhase::Idle {
            return;
        }
        self.reset_session();
        self.run.state.phase = CameraPhase::Idle;
        self.actions.push(CaptureAction::Unbind);
    }

    fn opened(&mut self, has_flash: bool, zoom_min: f32, zoom_max: f32) {
        let state = &mut self.run.state;
        state.has_flash = has_flash;
        state.zoom_min = zoom_min;
        state.zoom_max = zoom_max.min(15.0).max(zoom_min);
        state.zoom = 1.0_f32.clamp(state.zoom_min, state.zoom_max);
        state.flash = flash_plan(state.flash, has_flash);
        self.run.open = true;
        if self.run.state.flash && self.profile.is_video() {
            self.actions.push(CaptureAction::SetTorch { on: true });
        }
        self.enter_ready();
    }

    fn enter_ready(&mut self) {
        if self.phase() == CameraPhase::Opening && self.run.open && self.run.streaming {
            self.run.state.phase = CameraPhase::Ready;
        }
    }

    fn leave_ready(&mut self) {
        if self.phase() == CameraPhase::Ready && !self.run.starting && !(self.run.open && self.run.streaming) {
            self.run.state.phase = CameraPhase::Opening;
        }
    }

    fn fault(&mut self, kind: FaultKind) {
        if matches!(self.phase(), CameraPhase::Idle | CameraPhase::Failed) {
            return;
        }
        match recovery_step(self.run.recovery_spent, kind) {
            RecoveryAction::Restart => {
                self.run.recovery_spent = true;
                let lens = self.run.state.lens;
                self.reset_session();
                self.run.state.phase = CameraPhase::Opening;
                self.actions.push(CaptureAction::Restart { lens });
            }
            RecoveryAction::Fail => self.fail(if kind == FaultKind::Reset { CameraRefusal::Busy } else { CameraRefusal::ConfigureFailed }),
        }
    }

    fn fail(&mut self, reason: CameraRefusal) {
        self.reset_session();
        self.run.state.phase = CameraPhase::Failed;
        self.run.state.fault = Some(reason);
        self.actions.push(CaptureAction::Unbind);
    }

    fn photo(&mut self) {
        if self.profile.mode == CaptureMode::Photo && self.phase() == CameraPhase::Ready && !self.run.state.shooting {
            self.run.state.shooting = true;
            self.actions.push(CaptureAction::TakePhoto);
        }
    }

    fn start(&mut self) {
        if !self.profile.is_video() || self.phase() != CameraPhase::Ready || self.run.starting || self.run.state.shooting {
            return;
        }
        self.run.starting = true;
        self.run.started = false;
        self.run.discard = false;
        self.run.end = None;
        self.run.state.recorded_ms = 0;
        self.run.state.fault = None;
        self.run.state.refused_ms = None;
        self.actions.push(CaptureAction::BeginRecording);
    }

    fn end_recording(&mut self, discard: bool) {
        self.run.discard |= discard;
        self.run.state.phase = CameraPhase::Stopping;
        self.actions.push(CaptureAction::EndRecording { discard });
    }

    fn cancel(&mut self) {
        match self.phase() {
            CameraPhase::Recording => self.end_recording(true),
            CameraPhase::Stopping => self.run.discard = true,
            CameraPhase::Ready if self.run.starting => self.end_recording(true),
            _ => {}
        }
    }

    fn record_finished(&mut self, end: RecordEnd) {
        self.run.starting = false;
        self.run.end = Some(end);
        self.run.state.phase = CameraPhase::Stopping;
        if self.run.started && end != RecordEnd::Failed && !self.run.discard {
            self.actions.push(CaptureAction::ReadClip);
        } else {
            self.settle(None);
        }
    }

    fn settle(&mut self, duration_ms: Option<u64>) {
        let end = self.run.end.unwrap_or(RecordEnd::Failed);
        let verdict = record_verdict(self.run.started, false, self.run.discard, end, duration_ms, self.profile.min_take_ms);
        let (action, fault) = match verdict {
            RecordVerdict::Deliver => (CaptureAction::Deliver, None),
            RecordVerdict::Skip => (CaptureAction::Discard, self.run.state.fault.filter(|fault| *fault == CameraRefusal::InCall)),
            RecordVerdict::TooShort => (CaptureAction::Discard, Some(CameraRefusal::TooShort)),
            RecordVerdict::CameraNotReady => (CaptureAction::Discard, Some(CameraRefusal::NotReady)),
            RecordVerdict::Failed => (CaptureAction::Discard, Some(CameraRefusal::RecordFailed)),
        };
        self.actions.push(action);
        self.run.state.fault = fault;
        self.run.state.refused_ms = duration_ms.filter(|_| fault == Some(CameraRefusal::TooShort));
        self.run.state.recorded_ms = 0;
        self.run.starting = false;
        self.run.started = false;
        self.run.discard = false;
        self.run.end = None;
        if let Some(lens) = self.run.pending_lens.take() {
            self.rebind(lens);
        } else {
            self.run.state.phase = CameraPhase::Opening;
            self.enter_ready();
        }
    }

    fn rebind(&mut self, lens: CameraLens) {
        self.run.state.lens = lens;
        self.run.open = false;
        self.run.streaming = false;
        self.run.state.phase = CameraPhase::Opening;
        if self.profile.mode == CaptureMode::Photo {
            self.run.state.flash = false;
        }
        self.actions.push(CaptureAction::Bind { lens });
    }

    fn set_lens(&mut self, lens: CameraLens) {
        if self.run.state.shooting || self.phase() == CameraPhase::Stopping {
            self.run.pending_lens = (lens != self.run.state.lens).then_some(lens);
            return;
        }
        if self.run.starting && self.phase() == CameraPhase::Ready {
            if lens != self.run.state.lens {
                self.run.pending_lens = Some(lens);
                self.end_recording(true);
            }
            return;
        }
        match self.phase() {
            CameraPhase::Idle | CameraPhase::Failed => self.run.state.lens = lens,
            CameraPhase::Opening | CameraPhase::Ready if lens != self.run.state.lens => self.rebind(lens),
            CameraPhase::Recording if lens != self.run.state.lens => {
                self.run.pending_lens = Some(lens);
                self.end_recording(true);
            }
            _ => {}
        }
    }

    fn set_flash(&mut self, on: bool) {
        let enabled = flash_plan(on, self.run.state.has_flash);
        if enabled == self.run.state.flash {
            return;
        }
        self.run.state.flash = enabled;
        if self.profile.is_video() {
            self.actions.push(CaptureAction::SetTorch { on: enabled });
        }
    }

    fn zoom_by(&mut self, factor: f32) {
        if !matches!(self.phase(), CameraPhase::Ready | CameraPhase::Recording | CameraPhase::Stopping) {
            return;
        }
        let state = &self.run.state;
        let ratio = zoom_target(state.zoom, factor, state.zoom_min, state.zoom_max);
        if ratio != state.zoom {
            self.run.state.zoom = ratio;
            self.actions.push(CaptureAction::SetZoom { ratio });
        }
    }
}
