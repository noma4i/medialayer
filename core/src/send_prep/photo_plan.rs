use super::photo::{photo_turn, PhotoTurn};
use crate::ffi::{decode_cost, PixelSize};

/// A rectangle in fractions 0.0 to 1.0 of the upright photo.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct CropFractions {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What the app asks of a photo besides making it upright.
/// `max_size` fits the result into a rectangle without enlarging, `output_size` gives the exact size and wins over `max_size`.
/// `crop` is taken from the upright photo before `mirror`, `mirror` flips the cropped result horizontally.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct PhotoEdit {
    pub max_size: Option<PixelSize>,
    pub crop: Option<CropFractions>,
    pub output_size: Option<PixelSize>,
    pub mirror: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The whole plan of a photo: turn it upright, take `crop` of the upright pixels, scale to `output`, flip when `mirror`.
/// `needs_draw` is true when the crop, the mirror or the output size ask for a last draw after the photo is upright. `cost` is the memory asked of the Ledger:
/// the source raster (twice when the photo is turned) and the output raster when a last draw is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct PhotoPlan {
    pub turn: PhotoTurn,
    pub upright: PixelSize,
    pub crop: PixelRect,
    pub mirror: bool,
    pub output: PixelSize,
    pub needs_draw: bool,
    pub cost: u64,
}

/// The one plan of a photo edit, in the order turn, crop, mirror, fit. No plan for a photo without size or above the pixel limit,
/// a crop that is not a number or holds no pixel, a size of zero or an output above the pixel limit.
#[uniffi::export]
pub fn photo_plan(width: u32, height: u32, exif_orientation: u32, edit: PhotoEdit) -> Option<PhotoPlan> {
    let raster = decode_cost(width, height)?;
    let turn = photo_turn(exif_orientation);
    let turned = turn.rotation_degrees != 0 || turn.flip_horizontal;
    let (upright_width, upright_height) = if turn.rotation_degrees % 180 == 90 { (height, width) } else { (width, height) };
    let crop = match edit.crop {
        None => PixelRect { x: 0, y: 0, width: upright_width, height: upright_height },
        Some(fractions) => {
            let (x, width) = span(fractions.x, fractions.width, upright_width)?;
            let (y, height) = span(fractions.y, fractions.height, upright_height)?;
            PixelRect { x, y, width, height }
        }
    };
    let output = match (edit.output_size, edit.max_size) {
        (Some(exact), _) => exact,
        (None, Some(limit)) => fitted(crop.width, crop.height, limit)?,
        (None, None) => PixelSize { width: crop.width, height: crop.height },
    };
    if output.width == 0 || output.height == 0 {
        return None;
    }
    let whole = crop.width == upright_width && crop.height == upright_height;
    let needs_draw = !whole || edit.mirror || output.width != crop.width || output.height != crop.height;
    let kept = if turned { raster * 2 } else { raster };
    let cost = if needs_draw { kept + decode_cost(output.width, output.height)? } else { kept };
    Some(PhotoPlan { turn, upright: PixelSize { width: upright_width, height: upright_height }, crop, mirror: edit.mirror, output, needs_draw, cost })
}

fn span(start: f64, length: f64, total: u32) -> Option<(u32, u32)> {
    if !start.is_finite() || !length.is_finite() {
        return None;
    }
    let edge = |fraction: f64| (fraction.clamp(0.0, 1.0) * f64::from(total)).round() as u32;
    let (first, last) = (edge(start), edge(start + length));
    (last > first).then_some((first, last - first))
}

fn fitted(width: u32, height: u32, limit: PixelSize) -> Option<PixelSize> {
    if limit.width == 0 || limit.height == 0 {
        return None;
    }
    let scale = (f64::from(limit.width) / f64::from(width)).min(f64::from(limit.height) / f64::from(height)).min(1.0);
    let side = |value: u32, most: u32| ((f64::from(value) * scale).round() as u32).clamp(1, most);
    Some(PixelSize { width: side(width, limit.width.min(width)), height: side(height, limit.height.min(height)) })
}
