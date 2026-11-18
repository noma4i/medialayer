use super::profile::{CameraLens, CaptureProfile};
use super::refusal::CameraRefusal;
use super::rules::{FaultKind, RecordEnd};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CameraPhase {
    Idle,
    Opening,
    Ready,
    Recording,
    Stopping,
    Failed,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TakeOutcome { Captured { take: u64 }, Refused { reason: CameraRefusal }, Lost { reason: CameraRefusal } }

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
    pub take: Option<u64>,
    pub outcome: Option<TakeOutcome>,
}

/// The state of a camera and what the machine remembers between steps. The platform keeps one run per camera and replaces it with the step result.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CaptureRun {
    pub state: CameraState,
    pub open: bool,
    pub streaming: bool,
    pub audio_flowing: Option<bool>,
    pub starting: bool,
    pub started: bool,
    pub discard: bool,
    pub end: Option<RecordEnd>,
    pub recovery_spent: bool,
    pub pending_lens: Option<CameraLens>,
    /// The lens the running or last take was started on. A delivered clip carries this lens, not `state.lens`, which a lens change may already have moved on.
    pub take_lens: CameraLens,
}

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Enum)]
pub enum CaptureEvent {
    DemandOn,
    DemandOff,
    LayerClosed,
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
    Revoked { reason: CameraRefusal },
    AudioFlow { flowing: bool },
    RecordFinished { end: RecordEnd },
    RecordSettled { duration_ms: Option<u64> },
    SetLens { lens: CameraLens },
    SetFlash { on: bool },
    ZoomBy { factor: f32 },
}

/// A delivered clip belongs to its take: the platform keeps the clip of every `Captured { take }` until the app fetches it with `clip(take)` or the layer closes (`LayerClosed`).
/// Starting the next take, `DemandOff`, a lens change and `Unbind` never drop it. Only a running or unsettled take is thrown away by `Unbind` and `Restart`.
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
        state: CameraState { phase: CameraPhase::Idle, lens, flash: false, has_flash: false, zoom: 1.0, zoom_min: 1.0, zoom_max: 1.0, recorded_ms: 0, shooting: false, fault: None, refused_ms: None, take: None, outcome: None },
        open: false,
        streaming: false,
        audio_flowing: None,
        starting: false,
        started: false,
        discard: false,
        end: None,
        recovery_spent: false,
        pending_lens: None,
        take_lens: lens,
    }
}

/// The one machine of a camera: the next run and the actions for a platform event or an app command. A command that does not fit the phase changes nothing.
#[uniffi::export]
pub fn capture_step(run: CaptureRun, event: CaptureEvent, profile: CaptureProfile) -> CaptureStep {
    super::machine::step(run, event, profile)
}
