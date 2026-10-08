use crate::api::MediaError;
use crate::blurhash::{decode, BlurHashError};

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
pub fn image_pixel_size(width: f64, height: f64, scale: f64, declared: Option<PixelSize>) -> Option<PixelSize> {
    if declared.is_some() {
        return declared;
    }
    let pixels = |points: f64| (points * scale / PIXEL_STEP).ceil() * PIXEL_STEP;
    let valid = |value: f64| value.is_finite() && value > 0.0;
    if !(valid(width) && valid(height) && valid(scale)) {
        return None;
    }
    Some(PixelSize { width: pixels(width).min(MAX_PIXELS) as u32, height: pixels(height).min(MAX_PIXELS) as u32 })
}

#[uniffi::export]
pub fn decode_blurhash(hash: String, width: u32, height: u32, punch: f32) -> Result<Vec<u8>, MediaError> {
    Ok(decode(&hash, width, height, punch)?)
}

#[cfg(test)]
mod image_tests;
