use super::strategy::CacheStrategy;
use crate::pages::Keyed;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Image,
    Video,
    Audio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub kind: SourceKind,
    pub strategy: CacheStrategy,
}

impl Keyed for Source {
    fn key(&self) -> &str {
        &self.url
    }
}
