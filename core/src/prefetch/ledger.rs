use std::collections::{HashMap, HashSet};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counters {
    pub requested: u64,
    pub started: u64,
    pub completed: u64,
    pub deduped: u64,
    pub failed: u64,
    pub cancelled: u64,
}

#[derive(Debug, Clone, Copy)]
struct DedupeEntry {
    timestamp: Duration,
    depth: Duration,
}

#[derive(Debug, Clone)]
pub struct PrefetchLedger {
    dedupe_window: Duration,
    max_entries: usize,
    pub counters: Counters,
    dedupe: HashMap<String, DedupeEntry>,
    queued: HashMap<String, u32>,
    pending_cancellations: HashSet<String>,
}

impl PrefetchLedger {
    pub fn new(dedupe_window: Duration, max_entries: usize) -> Self {
        Self {
            dedupe_window,
            max_entries,
            counters: Counters::default(),
            dedupe: HashMap::new(),
            queued: HashMap::new(),
            pending_cancellations: HashSet::new(),
        }
    }

    pub fn set_limits(&mut self, dedupe_window: Duration, max_entries: usize) {
        self.dedupe_window = dedupe_window;
        self.max_entries = max_entries;
        self.trim_dedupe(Duration::ZERO);
    }

    pub fn dedupe_count(&self) -> usize {
        self.dedupe.len()
    }

    pub fn admit(&mut self, key: &str, depth: Duration, now: Duration) -> bool {
        let previous = self.dedupe.get(key).copied();
        let is_covered =
            previous.is_some_and(|entry| now.saturating_sub(entry.timestamp) < self.dedupe_window && depth <= entry.depth);
        if is_covered {
            return false;
        }
        let covered_depth = previous.map_or(depth, |entry| depth.max(entry.depth));
        self.dedupe.insert(key.to_string(), DedupeEntry { timestamp: now, depth: covered_depth });
        self.trim_dedupe(now);
        true
    }

    pub fn forget(&mut self, key: &str) {
        self.dedupe.remove(key);
    }

    pub fn enqueue(&mut self, key: &str) {
        *self.queued.entry(key.to_string()).or_insert(0) += 1;
    }

    pub fn begin(&mut self, key: &str) -> bool {
        let remaining = self.queued.get(key).copied().unwrap_or(1).saturating_sub(1);
        if remaining == 0 {
            self.queued.remove(key);
        } else {
            self.queued.insert(key.to_string(), remaining);
        }
        let is_cancelled = self.pending_cancellations.contains(key);
        if is_cancelled && remaining == 0 {
            self.pending_cancellations.remove(key);
        }
        is_cancelled
    }

    pub fn cancel(&mut self, key: &str, had_active: bool) -> bool {
        let had_queued = self.queued.get(key).copied().unwrap_or(0) > 0;
        if had_queued {
            self.pending_cancellations.insert(key.to_string());
        }
        self.dedupe.remove(key);
        if had_queued || had_active {
            self.counters.cancelled += 1;
        }
        had_queued
    }

    fn trim_dedupe(&mut self, now: Duration) {
        if self.dedupe.len() <= self.max_entries {
            return;
        }
        let window = self.dedupe_window;
        self.dedupe.retain(|_, entry| now.saturating_sub(entry.timestamp) <= window);
        while self.dedupe.len() > self.max_entries {
            let Some(oldest) = self.dedupe.iter().min_by_key(|(key, entry)| (entry.timestamp, key.as_str())).map(|(key, _)| key.clone())
            else {
                break;
            };
            self.dedupe.remove(&oldest);
        }
    }
}
