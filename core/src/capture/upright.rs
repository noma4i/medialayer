use crate::send_prep::VideoPlan;

/// What the platform read from a finished take: the stored pixel size, the turn its matrix asks for (a multiple of 90), and the bitrate and frame rate it was recorded at.
/// Only a mirrored take is re-encoded: the reflection matrix of an iOS front take breaks the server, so it is baked into the pixels. A plain turn (90, 180, 270) stays as the metadata tag, every player and the server read it.
/// `None` when `mirror` is false, whatever the turn: the take goes as it is.
/// With `mirror` a plan that re-encodes at the size the viewer sees (sides swapped on 90 and 270), the bitrate and frame rate of the take, and the turn to bake together with the mirror.
#[uniffi::export]
pub fn capture_upright_plan(width: u32, height: u32, rotation_degrees: i32, mirror: bool, bitrate: u32, frame_rate: f32) -> Option<VideoPlan> {
    let rotation = rotation_degrees.rem_euclid(360) as u32;
    if !mirror {
        return None;
    }
    let (width, height) = if rotation % 180 == 90 { (height, width) } else { (width, height) };
    Some(VideoPlan { reencode: true, width, height, bitrate, frame_rate: frame_rate.round().max(1.0) as u32, rotation_degrees: rotation })
}
