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
    #[error("tone: {detail}")]
    Tone { detail: String },
    #[error("invalid suspension: {detail}")]
    InvalidSuspension { detail: String },
    #[error("overloaded: {detail}")]
    Overloaded { detail: String },
    #[error("the media layer is closed")]
    Closed,
    #[error("unexpected: {detail}")]
    Unexpected { detail: String },
}

impl From<LimitsError> for MediaError {
    fn from(error: LimitsError) -> Self {
        MediaError::InvalidLimits { detail: error.to_string() }
    }
}

impl From<crate::session::SuspensionError> for MediaError {
    fn from(error: crate::session::SuspensionError) -> Self {
        MediaError::InvalidSuspension { detail: error.to_string() }
    }
}

impl From<crate::analysis::ToneError> for MediaError {
    fn from(error: crate::analysis::ToneError) -> Self {
        MediaError::Tone { detail: error.to_string() }
    }
}

impl From<uniffi::UnexpectedUniFFICallbackError> for MediaError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        MediaError::Unexpected { detail: error.reason }
    }
}
