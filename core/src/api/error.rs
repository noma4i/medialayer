use crate::limits::LimitsError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum MediaError {
    #[error("invalid limits: {detail}")]
    InvalidLimits { detail: String },
    #[error("invalid spec: {detail}")]
    InvalidSpec { detail: String },
    #[error("fetch failed: {detail}")]
    Fetch { detail: String },
    #[error("blurhash: {detail}")]
    BlurHash { detail: String },
    #[error("unexpected: {detail}")]
    Unexpected { detail: String },
}

impl From<LimitsError> for MediaError {
    fn from(error: LimitsError) -> Self {
        MediaError::InvalidLimits { detail: error.to_string() }
    }
}

impl From<uniffi::UnexpectedUniFFICallbackError> for MediaError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        MediaError::Unexpected { detail: error.reason }
    }
}
