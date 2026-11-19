mod coverage;
mod format;
mod profile;
mod refusal;
mod rules;
mod state;
mod machine;

pub use coverage::{audio_coverage_refusal, coverage_begin, coverage_finish, coverage_push, AudioCoverage, AudioInterval, CoverageConfig, CoverageScan, CoverageVerdict, VideoBounds};
pub use format::{format_candidates, pick_format, FormatCandidate, FormatChoice, FpsRange};
pub use profile::{capture_profile, check_capture_profile, stabilization_mode, CameraLens, CaptureKind, CaptureMode, CaptureProfile, VideoStabilization};
pub use refusal::{camera_refusal_text, capture_cost, capture_file_name, is_capture_orphan, shot_cost, CameraRefusal, CaptureFileKind};
pub use rules::{capture_tick_ms, flash_plan, frame_gate, microphone_lost, record_verdict, recovery_step, zoom_target, FaultKind, RecordEnd, RecordVerdict, RecoveryAction};
pub use state::{capture_run, capture_step, CameraPhase, CameraState, CaptureAction, CaptureEvent, CaptureRun, CaptureStep, TakeOutcome};

#[cfg(test)]
mod capture_tests;
#[cfg(test)]
mod coverage_tests;
