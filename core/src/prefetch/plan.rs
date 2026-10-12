use crate::hls::Chunk;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Plan {
    pub chunks: Vec<Chunk>,
    pub headers: HashMap<String, String>,
}
