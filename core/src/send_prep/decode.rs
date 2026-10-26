use super::photo::{photo_turn, PhotoTurn};
use crate::ffi::decode_cost;

/// How to decode a picture: the size to decode at, the turn that makes it upright, the size after the turn and the memory the decode asks of the Ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ImageDecodePlan {
    pub decode_width: u32,
    pub decode_height: u32,
    pub width: u32,
    pub height: u32,
    pub turn: PhotoTurn,
    pub cost: u64,
}

/// The one plan of a picture decode. The long side is cut to `max_side` (absent or 0 keeps the size), a picture is never enlarged.
/// The cost is one raster, two when the picture is turned. A size that is zero or above the pixel limit has no plan.
#[uniffi::export]
pub fn image_decode_plan(width: u32, height: u32, exif_orientation: u32, max_side: Option<u32>) -> Option<ImageDecodePlan> {
    if width == 0 || height == 0 {
        return None;
    }
    let long = u64::from(width.max(height));
    let (decode_width, decode_height) = match max_side.filter(|side| *side > 0 && u64::from(*side) < long) {
        Some(side) => {
            let cut = |value: u32| ((u64::from(value) * u64::from(side) + long / 2) / long).max(1) as u32;
            (cut(width), cut(height))
        }
        None => (width, height),
    };
    let raster = decode_cost(decode_width, decode_height)?;
    let turn = photo_turn(exif_orientation);
    let turned = turn.rotation_degrees != 0 || turn.flip_horizontal;
    let sideways = turn.rotation_degrees % 180 == 90;
    let (width, height) = if sideways { (decode_height, decode_width) } else { (decode_width, decode_height) };
    Some(ImageDecodePlan { decode_width, decode_height, width, height, turn, cost: if turned { raster * 2 } else { raster } })
}
