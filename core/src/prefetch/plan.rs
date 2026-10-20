use crate::hls::Chunk;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Plan {
    pub playlists: Vec<String>,
    pub chunks: Vec<Chunk>,
    pub headers: HashMap<String, String>,
}
