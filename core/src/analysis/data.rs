use super::swatch::{Dominant, Rgb, Swatch};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ToneSource {
    BlurHash,
    Pixels,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DominantColor {
    pub color: Rgb,
    pub share_permille: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToneData {
    pub source: ToneSource,
    pub cells: Vec<Rgb>,
    pub overall: Rgb,
    pub dominant: Vec<DominantColor>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct MediaTone {
    pub source: ToneSource,
    pub columns: u32,
    pub rows: u32,
    pub cells: Vec<Rgb>,
    pub band: f32,
    pub overall: Swatch,
    pub top: Swatch,
    pub rest: Swatch,
    pub dominant: Vec<Dominant>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToneError {
    #[error("image width and height must be above zero")]
    ZeroSize,
    #[error("pixel data length is not width x height x 4")]
    LengthMismatch,
    #[error("image of {pixels} pixels is above the limit of {limit}")]
    TooLarge { pixels: u64, limit: u64 },
    #[error("band must be above 0 and at most 1")]
    InvalidBand,
    #[error("blurhash: {0}")]
    BlurHash(String),
}
