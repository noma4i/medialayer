#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFocus {
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
    PermissionDenied,
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
}
#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    Connecting,
    Live,
    Interrupted,
    Ended,
}
#[derive(uniffi::Enum, Debug, Clone, PartialEq)]
pub enum AudioInput {
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
        answer: FocusAnswer,
    },
    MediaReset,
    Route {
        reason: RouteReason,
        had_private_output: bool,
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
        granted: bool,
    },
    OtherRecording {
        active: bool,
    },
    Silenced {
        id: u64,
    },
    Foreground {
        on: bool,
    },
}
#[derive(uniffi::Enum, Debug, Clone, PartialEq)]
pub enum AudioAction {
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
}
