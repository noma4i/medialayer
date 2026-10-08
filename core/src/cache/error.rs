#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CacheError {
    #[error("not enough space in the cache")]
    NotEnoughSpace,
    #[error("io error: {0}")]
    Io(String),
}

impl From<std::io::Error> for CacheError {
    fn from(error: std::io::Error) -> Self {
        CacheError::Io(error.to_string())
    }
}
