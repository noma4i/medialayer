use crate::api::is_local_url;
use crate::image_size::ImageSize;

/// What a local video or audio file says about itself, read without a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct MediaInfo {
    pub duration_ms: Option<u64>,
    pub frame: Option<ImageSize>,
}

/// The frame of a video as the screen shows it: a turn of 90 or 270 degrees swaps the sides. No frame when a side is zero.
#[uniffi::export]
pub fn display_frame_size(width: u32, height: u32, rotation_degrees: i32) -> Option<ImageSize> {
    if width == 0 || height == 0 {
        return None;
    }
    let turned = rotation_degrees.rem_euclid(180) == 90;
    Some(if turned { ImageSize { width: height, height: width } } else { ImageSize { width, height } })
}

/// Whether a url names a file the platform reads itself: only these have metadata without a download.
#[uniffi::export]
pub fn is_local_media_url(url: String) -> bool {
    is_local_url(&crate::api::clean_url(&url))
}

#[cfg(test)]
mod info_tests;
