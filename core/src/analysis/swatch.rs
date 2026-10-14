use crate::blurhash::{linear_to_srgb, srgb_to_linear};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct Swatch {
    pub color: Rgb,
    pub luminance: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct Dominant {
    pub color: Rgb,
    pub luminance: f64,
    pub share: f64,
}

const RED_WEIGHT: f64 = 0.2126;
const GREEN_WEIGHT: f64 = 0.7152;
const BLUE_WEIGHT: f64 = 0.0722;

pub fn linear(value: u8) -> f64 {
    static TABLE: OnceLock<[f64; 256]> = OnceLock::new();
    TABLE.get_or_init(|| std::array::from_fn(|level| srgb_to_linear(level as u32)))[usize::from(value)]
}

pub fn encode(linear: [f64; 3]) -> Rgb {
    Rgb { r: linear_to_srgb(linear[0]), g: linear_to_srgb(linear[1]), b: linear_to_srgb(linear[2]) }
}

pub fn luminance(color: Rgb) -> f64 {
    RED_WEIGHT * linear(color.r) + GREEN_WEIGHT * linear(color.g) + BLUE_WEIGHT * linear(color.b)
}

pub fn swatch(color: Rgb) -> Swatch {
    Swatch { color, luminance: luminance(color) }
}
