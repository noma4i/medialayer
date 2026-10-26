use crate::api::MediaError;
use crate::blurhash::{decode, BlurHashError};
mod counters;
mod hold;
pub use hold::{decode_cost, DecodeHold};
pub use counters::{ImageCounters, ImageSnapshot, ImageTotals, ImageWork};

impl From<BlurHashError> for MediaError {
    fn from(error: BlurHashError) -> Self {
        MediaError::BlurHash { detail: error.to_string() }
    }
}

const PIXEL_STEP: f64 = 8.0;
const MAX_PIXELS: f64 = 16_384.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

#[uniffi::export]
pub fn image_pixel_size(width: f64, height: f64, scale: f64, declared: Option<PixelSize>, crop: bool) -> Option<PixelSize> {
    let pixels = |points: f64| (points * scale / PIXEL_STEP).ceil() * PIXEL_STEP;
    let valid = |value: f64| value.is_finite() && value > 0.0;
    let (width, height) = if valid(width) && valid(height) && valid(scale) {
        (pixels(width).min(MAX_PIXELS), pixels(height).min(MAX_PIXELS))
    } else {
        let source = declared?;
        (f64::from(source.width), f64::from(source.height))
    };
    let Some(source) = declared else {
        return Some(PixelSize { width: width as u32, height: height as u32 });
    };
    if source.width == 0 || source.height == 0 { return None; }
    let (sw, sh) = (f64::from(source.width), f64::from(source.height));
    let ratio = if crop { (width / sw).max(height / sh) } else { (width / sw).min(height / sh) };
    let ratio = ratio.min(1.0).min(MAX_PIXELS / sw.max(sh));
    Some(PixelSize { width: (sw * ratio).ceil() as u32, height: (sh * ratio).ceil() as u32 })
}

#[uniffi::export]
pub fn decode_blurhash(hash: String, width: u32, height: u32, punch: f32) -> Result<Vec<u8>, MediaError> {
    Ok(decode(&hash, width, height, punch)?)
}

#[cfg(test)]
mod image_tests;
