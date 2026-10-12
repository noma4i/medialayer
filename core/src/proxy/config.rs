use crate::fetch::FetchPolicy;
use crate::limits::Limits;
use crate::memory::{InputLimits, Ledger};
use std::time::Duration;

const PLAYBACK_RESERVED: usize = 1;
const STREAM_CHUNK: usize = 64 * 1024;
const BLOCK_SIZE: u64 = 1024 * 1024;
const PLAYLIST_TTL: Duration = Duration::from_secs(2);
const MAX_WAITING: usize = 64;
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);
const PING_TIMEOUT: Duration = Duration::from_secs(1);
const MIN_BUFFER: usize = 4 * 1024;
const HOLD_CEILING: Duration = Duration::from_secs(3);
const PROBE_LIMIT: Duration = Duration::from_millis(300);

#[derive(Debug, Clone)]
pub struct Config {
    pub policy: FetchPolicy,
    pub max_concurrent: usize,
    pub playback_reserved: usize,
    pub stream_chunk: usize,
    pub chunk_size: u64,
    pub playlist_ttl: Duration,
    pub max_waiting: usize,
    pub max_playlist_bytes: u64,
    pub header_read_timeout: Duration,
    pub ping_timeout: Duration,
    pub ledger: Ledger,
    pub min_buffer: usize,
    pub namespace_headers: Vec<String>,
    pub chunks_ahead: u32,
    pub hold_ceiling: Duration,
    pub probe_limit: Duration,
}

impl Config {
    pub fn from_limits(limits: &Limits) -> Self {
        Config {
            policy: FetchPolicy::from_limits(limits),
            max_concurrent: limits.max_concurrent as usize,
            playback_reserved: PLAYBACK_RESERVED,
            stream_chunk: STREAM_CHUNK,
            chunk_size: BLOCK_SIZE,
            playlist_ttl: PLAYLIST_TTL,
            max_waiting: MAX_WAITING,
            max_playlist_bytes: InputLimits::default().playlist_bytes,
            header_read_timeout: HEADER_READ_TIMEOUT,
            ping_timeout: PING_TIMEOUT,
            ledger: Ledger::unbounded(),
            min_buffer: MIN_BUFFER,
            namespace_headers: Vec::new(),
            chunks_ahead: limits.playback_chunks_ahead,
            hold_ceiling: HOLD_CEILING,
            probe_limit: PROBE_LIMIT,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config::from_limits(&Limits::default())
    }
}
