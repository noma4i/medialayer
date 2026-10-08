use crate::api::MediaError;
use crate::blurhash::{decode, BlurHashError};

impl From<BlurHashError> for MediaError {
    fn from(error: BlurHashError) -> Self {
        MediaError::BlurHash { detail: error.to_string() }
    }
}

#[uniffi::export]
pub fn decode_blurhash(hash: String, width: u32, height: u32, punch: f32) -> Result<Vec<u8>, MediaError> {
    Ok(decode(&hash, width, height, punch)?)
}

#[cfg(test)]
mod image_tests;
