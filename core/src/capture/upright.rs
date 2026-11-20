use crate::send_prep::VideoPlan;

/// What the platform read from a finished take: the stored pixel size, the turn its matrix asks for (a multiple of 90), and the bitrate and frame rate it was recorded at.
/// The take is delivered upright with an identity matrix. `None` when the file already is (no turn, no mirror): it goes as it is.
/// Otherwise a plan that re-encodes at the size the viewer sees (sides swapped on 90 and 270), the bitrate and frame rate of the take, and the turn to apply; `mirror` is applied by the encoder in pixels.
#[uniffi::export]
pub fn capture_upright_plan(width: u32, height: u32, rotation_degrees: i32, mirror: bool, bitrate: u32, frame_rate: f32) -> Option<VideoPlan> {
    let rotation = rotation_degrees.rem_euclid(360) as u32;
    if rotation == 0 && !mirror {
        return None;
    }
    let (width, height) = if rotation % 180 == 90 { (height, width) } else { (width, height) };
    Some(VideoPlan { reencode: true, width, height, bitrate, frame_rate: frame_rate.round().max(1.0) as u32, rotation_degrees: rotation })
}
