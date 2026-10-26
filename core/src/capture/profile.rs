use crate::api::MediaError;
use crate::ffi::PixelSize;

const FPS_LEAST: u32 = 1;
const FPS_MOST: u32 = 60;
const VIDEO_LEAST_MS: u64 = 1_000;
const VIDEO_MOST_MS: u64 = 600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CaptureKind {
    Photo,
    Moment,
    Circle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CameraLens {
    Back,
    Front,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CaptureMode {
    Photo,
    Video,
}

/// How much the video is steadied. `Cinematic` is the strongest, `Off` asks for none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum VideoStabilization {
    Off,
    Standard,
    Cinematic,
}

/// What a camera place captures. A `Photo` profile has no video and no limit, a `Video` profile has a video size and a limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct CaptureProfile {
    pub mode: CaptureMode,
    pub video_size: Option<PixelSize>,
    pub photo_size: Option<PixelSize>,
    pub fps: u32,
    pub max_duration_ms: u64,
    pub min_take_ms: u64,
    pub audio: bool,
    pub mirror_front_video: bool,
    pub stabilization: VideoStabilization,
    pub photo_quality: u32,
    pub reduces_red_eye: bool,
}

/// The three profiles of the app: `Photo` (target 1080x1920, JPEG 90), `Moment` (1280x720, 10 s, 500 ms least take, sound) and `Circle` (720x1280, 60 s, sound, cinematic stabilization).
#[uniffi::export]
pub fn capture_profile(kind: CaptureKind) -> CaptureProfile {
    let base = CaptureProfile {
        mode: CaptureMode::Video,
        video_size: None,
        photo_size: None,
        fps: 30,
        max_duration_ms: 0,
        min_take_ms: 0,
        audio: true,
        mirror_front_video: false,
        stabilization: VideoStabilization::Off,
        photo_quality: 90,
        reduces_red_eye: false,
    };
    match kind {
        CaptureKind::Photo => CaptureProfile { mode: CaptureMode::Photo, photo_size: Some(PixelSize { width: 1080, height: 1920 }), audio: false, ..base },
        CaptureKind::Moment => CaptureProfile { video_size: Some(PixelSize { width: 1280, height: 720 }), max_duration_ms: 10_000, min_take_ms: 500, ..base },
        CaptureKind::Circle => CaptureProfile { video_size: Some(PixelSize { width: 720, height: 1280 }), max_duration_ms: 60_000, stabilization: VideoStabilization::Cinematic, ..base },
    }
}

/// The profile itself when it is valid. A side of zero, a frame rate outside 1 to 60, a video without a size, a video limit outside 1 second to 10 minutes,
/// a photo with a limit, a sound or a stabilization, a least take that is not below the limit and a quality above 100 are `MediaError::InvalidSpec`.
#[uniffi::export]
pub fn check_capture_profile(profile: CaptureProfile) -> Result<CaptureProfile, MediaError> {
    let refuse = |detail: &str| Err(MediaError::InvalidSpec { detail: format!("capture profile: {detail}") });
    let empty = |size: Option<PixelSize>| size.is_some_and(|size| size.width == 0 || size.height == 0);
    if empty(profile.video_size) || empty(profile.photo_size) {
        return refuse("a side is zero");
    }
    if !(FPS_LEAST..=FPS_MOST).contains(&profile.fps) {
        return refuse("the frame rate is outside 1 to 60");
    }
    if profile.photo_quality == 0 || profile.photo_quality > 100 {
        return refuse("the photo quality is outside 1 to 100");
    }
    match profile.mode {
        CaptureMode::Video => {
            if profile.video_size.is_none() {
                return refuse("a video has no size");
            }
            if !(VIDEO_LEAST_MS..=VIDEO_MOST_MS).contains(&profile.max_duration_ms) {
                return refuse("the video limit is outside 1 second to 10 minutes");
            }
            if profile.min_take_ms >= profile.max_duration_ms {
                return refuse("the least take is not below the limit");
            }
        }
        CaptureMode::Photo => {
            if profile.max_duration_ms != 0 || profile.min_take_ms != 0 || profile.audio || profile.video_size.is_some() || profile.stabilization != VideoStabilization::Off {
                return refuse("a photo has no limit, no least take, no sound, no video size and no stabilization");
            }
        }
    }
    Ok(profile)
}

/// The stabilization to set on the video connection: the requested one when the device offers it (`Off` always), otherwise `None` and the platform keeps its own.
#[uniffi::export]
pub fn stabilization_mode(requested: VideoStabilization, supported: Vec<VideoStabilization>) -> Option<VideoStabilization> {
    (requested == VideoStabilization::Off || supported.contains(&requested)).then_some(requested)
}
