use crate::api::MediaError;
use crate::memory::InputLimits;

/// Admission bounds of the typed input: queued entries, live owners and held payload bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct IngressLimits {
    pub entries: u32,
    pub owners: u32,
    pub bytes: u64,
}

impl Default for IngressLimits {
    fn default() -> Self {
        IngressLimits { entries: 256, owners: 256, bytes: InputLimits::default().response_bytes }
    }
}

impl IngressLimits {
    pub fn validate(&self) -> Result<(), MediaError> {
        if self.entries == 0 || self.owners == 0 || self.bytes == 0 {
            return Err(MediaError::InvalidLimits { detail: "ingress limits must be positive".to_string() });
        }
        Ok(())
    }
}
