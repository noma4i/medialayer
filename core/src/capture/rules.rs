const ZOOM_CEILING: f32 = 15.0;
const TICK_MS: u64 = 100;
/// How far the video may run ahead of the written sound before the microphone counts as lost. Owner number: 1000 ms.
pub const MICROPHONE_LOST_LAG_US: i64 = 1_000_000;

/// How a recording ended on the platform: by the app, by the limit, by a microphone that stopped delivering (handled as the limit), or by a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RecordEnd {
    Completed,
    LimitReached,
    MicrophoneLost,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RecordVerdict {
    Deliver,
    TooShort,
    Skip,
    CameraNotReady,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FaultKind {
    Reset,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RecoveryAction {
    Restart,
    Fail,
}

/// The zoom after `factor`: `current * factor` inside `min` to `min(max, 15)`. A factor that is not a finite positive number keeps the current zoom.
#[uniffi::export]
pub fn zoom_target(current: f32, factor: f32, min: f32, max: f32) -> f32 {
    if !factor.is_finite() || factor <= 0.0 || !current.is_finite() {
        return current;
    }
    (current * factor).clamp(min, max.min(ZOOM_CEILING).max(min))
}

/// The flash is on only when it was asked for and the camera has a flash unit.
#[uniffi::export]
pub fn flash_plan(requested: bool, has_unit: bool) -> bool {
    requested && has_unit
}

/// The verdict of a finished recording. Closed or thrown away is `Skip`, a recording that never started is `CameraNotReady`, a failed end or a missing length is `Failed`,
/// a length not above the least take is `TooShort`, anything else is `Deliver`. The end by the limit is a good end.
#[uniffi::export]
pub fn record_verdict(started: bool, closed: bool, discard: bool, end: RecordEnd, duration_ms: Option<u64>, min_take_ms: u64) -> RecordVerdict {
    if closed || discard {
        return RecordVerdict::Skip;
    }
    if !started {
        return RecordVerdict::CameraNotReady;
    }
    match (end, duration_ms) {
        (RecordEnd::Failed, _) | (_, None) => RecordVerdict::Failed,
        (_, Some(length)) if length <= min_take_ms => RecordVerdict::TooShort,
        _ => RecordVerdict::Deliver,
    }
}

/// Whether the microphone stopped delivering mid-take: the sound has started (`audio_end_us >= 0`) and the video runs ahead of it by more than the owner lag.
#[uniffi::export]
pub fn microphone_lost(audio_end_us: i64, video_end_us: i64) -> bool {
    audio_end_us >= 0 && video_end_us.saturating_sub(audio_end_us) > MICROPHONE_LOST_LAG_US
}

/// One restart after a reset of the camera service, then a failure. Any other fault is a failure. `spent` tells that the restart was used.
#[uniffi::export]
pub fn recovery_step(spent: bool, fault: FaultKind) -> RecoveryAction {
    if fault == FaultKind::Reset && !spent {
        RecoveryAction::Restart
    } else {
        RecoveryAction::Fail
    }
}

/// Whether a frame at `now_ms` passes a gate of `per_second` frames. The first frame passes, a clock that went back resets the mark.
#[uniffi::export]
pub fn frame_gate(last_ms: Option<u64>, now_ms: u64, per_second: f64) -> bool {
    let Some(last) = last_ms else { return true };
    if now_ms < last {
        return true;
    }
    per_second.is_finite() && per_second > 0.0 && (now_ms - last) as f64 >= 1000.0 / per_second
}

/// The step in milliseconds at which the length of a recording is published.
#[uniffi::export]
pub fn capture_tick_ms() -> u64 {
    TICK_MS
}
