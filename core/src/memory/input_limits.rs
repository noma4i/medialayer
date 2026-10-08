const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputLimits {
    pub playlist_bytes: u64,
    pub response_bytes: u64,
    pub blurhash_chars: u64,
}

impl Default for InputLimits {
    fn default() -> Self {
        Self { playlist_bytes: 2 * MIB, response_bytes: 16 * MIB, blurhash_chars: 200 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("input of {len} exceeds the limit of {limit}")]
pub struct TooLarge {
    pub len: u64,
    pub limit: u64,
}

pub fn bounded(len: u64, limit: u64) -> Result<(), TooLarge> {
    if len > limit {
        return Err(TooLarge { len, limit });
    }
    Ok(())
}
