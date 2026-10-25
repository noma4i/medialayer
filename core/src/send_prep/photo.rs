/// How to make a photo upright: turn clockwise by `rotation_degrees`, then mirror horizontally when `flip_horizontal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct PhotoTurn {
    pub rotation_degrees: u32,
    pub flip_horizontal: bool,
}

/// The one table of the eight EXIF orientations. An unknown tag leaves the photo as it is.
#[uniffi::export]
pub fn photo_turn(exif_orientation: u32) -> PhotoTurn {
    let (rotation_degrees, flip_horizontal) = match exif_orientation {
        2 => (0, true),
        3 => (180, false),
        4 => (180, true),
        5 => (90, true),
        6 => (90, false),
        7 => (270, true),
        8 => (270, false),
        _ => (0, false),
    };
    PhotoTurn { rotation_degrees, flip_horizontal }
}

/// The JPEG quality in 0.0 to 1.0. A value that is not a number gives 0.9.
#[uniffi::export]
pub fn jpeg_quality(quality: f64) -> f64 {
    if quality.is_nan() {
        return 0.9;
    }
    quality.clamp(0.0, 1.0)
}
