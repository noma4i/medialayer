#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlurHashError {
    #[error("blurhash is shorter than 6 characters")]
    TooShort,
    #[error("blurhash length does not match its component count")]
    LengthMismatch,
    #[error("blurhash has a character outside base83")]
    InvalidCharacter,
    #[error("blurhash component count is outside 1..9")]
    ComponentsOutOfRange,
    #[error("image width and height must be above zero")]
    ZeroSize,
    #[error("image size is too large")]
    SizeTooLarge,
    #[error("punch must be a finite number")]
    InvalidPunch,
}

pub fn components(_hash: &str) -> Result<(u32, u32), BlurHashError> {
    Err(BlurHashError::TooShort)
}

pub fn decode(_hash: &str, _width: u32, _height: u32, _punch: f32) -> Result<Vec<u8>, BlurHashError> {
    Err(BlurHashError::TooShort)
}
