#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioFocus {
    #[default]
    Exclusive,
    Duck,
    Mix,
}
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingKind {
    Voice,
    Video,
}
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum CallKind {
    CallKit { uuid: String },
    Telecom,
    Unmanaged,
}
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophoneAvailability {
    Available,
    OwnCall,
    ExternalCall,
    OtherApp,
    PermissionNotAsked,
    PermissionDenied,
    NoSound,
}
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrophonePermission {
    Granted,
    NotAsked,
    Denied,
}
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingRefusal {
    InCall,
    Interrupted,
    MicrophoneSilent,
    PermissionDenied,
    MicrophoneBusy,
}
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusChange {
    Gain,
    LossTransient,
    LossTransientDuck,
    Loss,
}
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusAnswer {
    Granted,
    Delayed,
    Failed,
}
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteReason {
    OldDeviceUnavailable,
    NewDeviceAvailable,
    CategoryChange,
    Override,
    Other,
}
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppliedResult {
    Ok,
    Busy,
    Failed { code: i64 },
}
#[derive(uniffi::Enum, Debug, Clone, PartialEq, Eq)]
pub enum SessionWant {
    Playback { focus: AudioFocus },
    Recording { kind: RecordingKind },
    Call { kind: CallKind, speaker: bool },
}
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    Capture,
    Interrupted,
    FocusLost,
    OutputLost,
    MediaReset,
    InCall,
    Suspended,
}
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    Connecting,
    Live,
    Interrupted,
    Ended,
}
/// The output a call's audio goes to, as the platform applied it.
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallRoute { Speaker, Receiver, Other, Failed }
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureSource { Session, Focus, Speaker, Device, Recorder, Camera, File }
#[derive(uniffi::Record, Debug, Clone, PartialEq, Eq)]
pub struct NativeFailure { pub source: FailureSource, pub domain: String, pub code: i64, pub message: String }
#[derive(uniffi::Enum, Debug, Clone, PartialEq)]
pub enum AudioInput {
    Failure { failure: NativeFailure },
    VolumeReadStarted {
        id: u64,
    },
    VolumeReadEnded {
        id: u64,
    },
    Audible {
        slot: u64,
        on: bool,
    },
    Capture {
        id: u64,
        kind: RecordingKind,
    },
    CaptureEnded {
        id: u64,
    },
    /// A video capture began (`taking`) or ended a take. While a take runs a voice capture is refused busy; while it does not the voice takes the microphone from the preview.
    CaptureTake {
        id: u64,
        taking: bool,
    },
    CallStarted {
        id: u64,
        kind: CallKind,
        speaker: bool,
    },
    CallSpeaker {
        id: u64,
        speaker: bool,
    },
    CallEnded {
        id: u64,
    },
    CallSystemActivated {
        id: u64,
    },
    CallSystemDeactivated {
        id: u64,
    },
    Applied {
        generation: u64,
        result: AppliedResult,
    },
    InterruptionBegan,
    InterruptionEnded {
        should_resume: bool,
    },
    ResumptionRecommendation {
        should_resume: bool,
    },
    Focus {
        change: FocusChange,
    },
    FocusRequest {
        generation: u64,
        answer: FocusAnswer,
    },
    MediaReset,
    Route {
        reason: RouteReason,
        lost_private_output: bool,
    },
    Noisy,
    ObservedCalls {
        uuids: Vec<String>,
    },
    PhoneCall {
        active: bool,
    },
    CommunicationMode {
        active: bool,
    },
    Permission {
        state: MicrophonePermission,
    },
    OtherRecording {
        active: bool,
    },
    Silenced {
        id: u64,
    },
    InputOpened {
        id: u64,
        kind: RecordingKind,
    },
    InputClosed { id: u64 },
    NoSound {
        id: u64,
        generation: u64,
        on: bool,
    },
    RestoreFailed {
        id: u64,
        generation: u64,
    },
    Foreground {
        on: bool,
    },
}
#[derive(uniffi::Enum, Debug, Clone, PartialEq)]
pub enum AudioAction {
    AwaitPaused { barrier: u64 },
    Failure { failure: NativeFailure },
    Route { speaker: bool },
    Revoke {
        id: u64,
        reason: RecordingRefusal,
    },
    Refuse {
        id: u64,
        reason: RecordingRefusal,
    },
    PauseAll {
        reason: PauseReason,
    },
    ForgetPaused,
    CallHold {
        on: bool,
    },
    Deactivate {
        generation: u64,
    },
    Apply {
        generation: u64,
        want: SessionWant,
        activate: bool,
    },
    Admit {
        id: u64,
    },
    ResumePaused,
    VolumeMultiplier {
        value: f32,
    },
    CallState {
        id: u64,
        state: CallState,
    },
    Availability {
        value: MicrophoneAvailability,
    },
    RestoreInput {
        id: u64,
        kind: RecordingKind,
        generation: u64,
    },
    StopInput {
        id: u64,
        generation: u64,
    },
}
