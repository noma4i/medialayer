use super::clock::{Clock, SystemClock};
use super::entry::Kind;
use crate::limits::EvictionStrategy;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;
const DEFAULT_PERSIST_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct Options {
    pub kind_priority: [Kind; 3],
    pub overrides: HashMap<Kind, EvictionStrategy>,
    pub clock: Arc<dyn Clock>,
    pub chunk_size: usize,
    pub persist_delay: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            kind_priority: [Kind::Image, Kind::Stream, Kind::Audio],
            overrides: HashMap::new(),
            clock: Arc::new(SystemClock),
            chunk_size: DEFAULT_CHUNK_SIZE,
            persist_delay: DEFAULT_PERSIST_DELAY,
        }
    }
}
