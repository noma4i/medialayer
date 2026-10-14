mod codec;
mod data;
mod grid;
mod query;
mod swatch;

pub use data::{DominantColor, MediaTone, ToneData, ToneError, ToneSource};
pub use query::valid_band;
pub use grid::{analyze, analyze_blurhash, COLUMNS, DOMINANT, ROWS};
pub use swatch::{luminance, Dominant, Rgb, Swatch};

#[cfg(test)]
mod analysis_tests;
