use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Image,
    Stream,
    Audio,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Image, Kind::Stream, Kind::Audio];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn code(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Stream => "stream",
            Kind::Audio => "audio",
        }
    }

    pub fn from_code(code: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|kind| kind.code() == code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub size: u64,
    pub kind: Kind,
    pub created: u64,
    pub last_access: u64,
    pub hits: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KindStats {
    pub bytes: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub total_bytes: u64,
    pub file_count: u64,
    pub per_kind: BTreeMap<Kind, KindStats>,
    pub evictions: u64,
    pub max_hold: std::time::Duration,
    pub open_writers: u64,
    pub eviction_backlog: u64,
}
