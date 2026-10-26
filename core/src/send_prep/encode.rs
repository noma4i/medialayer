use super::photo::{photo_turn, PhotoTurn};
use crate::ffi::decode_cost;

/// The file format of an encoded image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ImageFormat {
    Jpeg,
    Png,
}

/// How to encode an image held in memory: the turn that makes it upright, whether the transparency is flattened onto white
/// (a JPEG has none) and the memory asked of the Ledger: one raster for the encoder and one more for each redraw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct EncodePlan {
    pub turn: PhotoTurn,
    pub flatten: bool,
    pub cost: u64,
}

/// The one plan of an image encode. A size that is zero or above the pixel limit has no plan.
#[uniffi::export]
pub fn image_encode_plan(width: u32, height: u32, exif_orientation: u32, has_alpha: bool, format: ImageFormat) -> Option<EncodePlan> {
    let raster = decode_cost(width, height)?;
    let turn = photo_turn(exif_orientation);
    let turned = turn.rotation_degrees != 0 || turn.flip_horizontal;
    let flatten = has_alpha && format == ImageFormat::Jpeg;
    Some(EncodePlan { turn, flatten, cost: raster * (1 + u64::from(turned) + u64::from(flatten)) })
}
