mod decode;

pub use decode::{components, decode, BlurHashError};
pub(crate) use decode::{linear_to_srgb, srgb_to_linear};

#[cfg(test)]
mod decode_tests;
