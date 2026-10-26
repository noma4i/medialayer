use super::profile::CaptureProfile;
use crate::ffi::{decode_cost, PixelSize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CameraRefusal {
    PermissionDenied,
    NoCamera,
    Busy,
    ConfigureFailed,
    NotReady,
    TooShort,
    RecordFailed,
    PhotoFailed,
    Memory,
}

/// The text of a refusal, the same on both platforms.
#[uniffi::export]
pub fn camera_refusal_text(reason: CameraRefusal) -> String {
    match reason {
        CameraRefusal::PermissionDenied => "the camera or microphone permission is not granted",
        CameraRefusal::NoCamera => "the device has no camera for this side",
        CameraRefusal::Busy => "the camera is used by another app or call",
        CameraRefusal::ConfigureFailed => "the camera could not be set up",
        CameraRefusal::NotReady => "the camera is not ready",
        CameraRefusal::TooShort => "the recording is too short",
        CameraRefusal::RecordFailed => "the recording failed",
        CameraRefusal::PhotoFailed => "the photo could not be taken",
        CameraRefusal::Memory => "there is not enough memory for the camera",
    }
    .to_string()
}

const FRAMES_IN_FLIGHT_VIDEO: u64 = 3;
const FRAMES_IN_FLIGHT_PHOTO: u64 = 2;
const PREVIEW_FALLBACK: PixelSize = PixelSize { width: 1920, height: 1080 };

/// The memory a running session asks of the Ledger: a few frames of the video size (or of the photo size for a photo profile) at 4 bytes a pixel.
#[uniffi::export]
pub fn capture_cost(profile: CaptureProfile) -> u64 {
    let (size, frames) = match profile.video_size {
        Some(size) => (size, FRAMES_IN_FLIGHT_VIDEO),
        None => (profile.photo_size.unwrap_or(PREVIEW_FALLBACK), FRAMES_IN_FLIGHT_PHOTO),
    };
    decode_cost(size.width, size.height).unwrap_or(0).saturating_mul(frames)
}

/// The memory one photo asks of the Ledger: its raster, twice because the turn to upright draws a second one. No cost for a size of zero or above the pixel limit.
#[uniffi::export]
pub fn shot_cost(width: u32, height: u32) -> Option<u64> {
    decode_cost(width, height).map(|raster| raster * 2)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CaptureFileKind {
    Photo,
    Clip,
}

const FILE_PREFIX: &str = "capture-";

/// The name of a captured file in the capture directory: `capture-<id>.jpg` or `capture-<id>.mp4`.
#[uniffi::export]
pub fn capture_file_name(kind: CaptureFileKind, id: String) -> String {
    let extension = match kind {
        CaptureFileKind::Photo => "jpg",
        CaptureFileKind::Clip => "mp4",
    };
    format!("{FILE_PREFIX}{id}.{extension}")
}

/// Whether a file name in the capture directory is a leftover of a capture: every name this module makes.
#[uniffi::export]
pub fn is_capture_orphan(name: String) -> bool {
    name.strip_prefix(FILE_PREFIX).is_some_and(|rest| rest.ends_with(".jpg") || rest.ends_with(".mp4"))
}
