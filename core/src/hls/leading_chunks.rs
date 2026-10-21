#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub segment: String,
    pub init: Option<String>,
}
