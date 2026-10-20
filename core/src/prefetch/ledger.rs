use super::plan::Plan;
use crate::hls::Chunk;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
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

#[derive(Debug, Clone)]
struct Held {
    chunk: usize,
    url: String,
    bytes: u64,
}

#[derive(Debug, Clone)]
struct Entry {
    stamp: Duration,
    loaded: usize,
    complete: bool,
    plan: Option<Arc<Plan>>,
    held: Vec<Held>,
    inits: HashSet<String>,
    played: bool,
    distance: usize,
    blocked_at: Option<u64>,
}

impl Entry {
    fn new(now: Duration) -> Entry {
        Entry { stamp: now, loaded: 0, complete: false, plan: None, held: Vec::new(), inits: HashSet::new(), played: false, distance: usize::MAX, blocked_at: None }
    }

    fn held_bytes(&self) -> u64 {
        if self.played {
            0
        } else {
            self.held.iter().map(|held| held.bytes).sum()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Evicted {
    pub urls: Vec<String>,
    pub headers: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NextLoad {
    Done,
    NeedPlan,
    Chunk { index: usize, chunk: Chunk, init_loaded: bool },
}

#[derive(Debug, Clone)]
pub struct PrefetchLedger {
    dedupe_window: Duration,
    max_entries: usize,
    pub counters: Counters,
    entries: HashMap<String, Entry>,
    held_bytes: u64,
    chunk_estimate: u64,
    epoch: u64,
    queued: HashMap<String, u32>,
    pending_cancellations: std::collections::HashSet<String>,
}

impl PrefetchLedger {
    pub fn new(dedupe_window: Duration, max_entries: usize) -> Self {
        Self {
            dedupe_window,
            max_entries,
            counters: Counters::default(),
            entries: HashMap::new(),
            held_bytes: 0,
            chunk_estimate: 0,
            epoch: 0,
            queued: HashMap::new(),
            pending_cancellations: HashSet::new(),
        }
    }

    pub fn set_limits(&mut self, dedupe_window: Duration, max_entries: usize) {
        self.dedupe_window = dedupe_window;
        self.max_entries = max_entries;
        self.epoch += 1;
        self.trim(Duration::ZERO);
    }

    pub fn forget_loaded(&mut self) {
        for entry in self.entries.values_mut() {
            *entry = Entry::new(entry.stamp);
        }
        self.held_bytes = 0;
        self.chunk_estimate = 0;
        self.epoch += 1;
    }

    pub fn dedupe_count(&self) -> usize {
        self.entries.len()
    }

    pub fn held_bytes(&self) -> u64 {
        self.held_bytes
    }

    pub fn loaded_chunks(&self, key: &str) -> usize {
        self.entries.get(key).map_or(0, |entry| entry.loaded)
    }

    pub(super) fn needed(&mut self, key: &str, target: usize, now: Duration, entering: bool) -> bool {
        let window = self.dedupe_window;
        let epoch = self.epoch;
        let entry = self.entries.entry(key.to_string()).or_insert_with(|| Entry::new(now));
        if entering && now.saturating_sub(entry.stamp) >= window {
            self.held_bytes -= entry.held_bytes();
            *entry = Entry::new(now);
        }
        let complete = entry.complete || (target != usize::MAX && entry.plan.as_ref().is_some_and(|plan| plan.chunks.len() <= entry.loaded));
        let blocked = entry.blocked_at == Some(epoch);
        let needed = entry.loaded < target && !complete && !blocked;
        if needed {
            entry.stamp = now;
        }
        self.trim(now);
        needed
    }

    pub(super) fn record_unit(&mut self, key: &str) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.loaded = entry.loaded.max(1);
        }
    }

    pub(super) fn record_complete(&mut self, key: &str) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.complete = true;
        }
    }

    pub(super) fn set_distance(&mut self, key: &str, distance: usize) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.distance = distance;
        }
    }

    pub(super) fn mark_played(&mut self, key: &str) {
        let Some(entry) = self.entries.get_mut(key) else { return };
        if entry.played {
            return;
        }
        self.held_bytes -= entry.held_bytes();
        entry.played = true;
        self.epoch += 1;
    }

    pub(super) fn set_plan(&mut self, key: &str, plan: Plan) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.plan = Some(Arc::new(plan));
        }
    }

    pub(super) fn step(&self, key: &str, target: usize) -> NextLoad {
        let Some(entry) = self.entries.get(key) else { return NextLoad::Done };
        let Some(plan) = &entry.plan else { return NextLoad::NeedPlan };
        if entry.loaded >= target || entry.loaded >= plan.chunks.len() {
            return NextLoad::Done;
        }
        let chunk = plan.chunks[entry.loaded].clone();
        let init_loaded = chunk.init.as_ref().is_none_or(|init| entry.inits.contains(init));
        NextLoad::Chunk { index: entry.loaded, chunk, init_loaded }
    }

    pub(super) fn make_room(&mut self, key: &str, distance: usize, budget: u64) -> Option<Vec<Evicted>> {
        let estimate = self.chunk_estimate;
        let mut evicted = Vec::new();
        while self.held_bytes + estimate > budget {
            let Some(farther) = self.farthest(Some(key)).filter(|(_, entry_distance)| *entry_distance > distance) else {
                if estimate == 0 {
                    break;
                }
                self.block(key);
                return None;
            };
            evicted.push(self.evict_entry(&farther.0));
        }
        Some(evicted)
    }

    pub(super) fn record_chunk(&mut self, key: &str, index: usize, parts: Vec<(String, u64)>, init: Option<String>, budget: u64) -> (Vec<Evicted>, bool) {
        let Some(entry) = self.entries.get_mut(key) else { return (Vec::new(), false) };
        let bytes: u64 = parts.iter().map(|(_, bytes)| bytes).sum();
        let replaced: u64 = entry.held.iter().filter(|held| held.chunk == index).map(|held| held.bytes).sum();
        entry.held.retain(|held| held.chunk != index);
        if !entry.played {
            self.held_bytes -= replaced;
        }
        entry.held.extend(parts.into_iter().map(|(url, bytes)| Held { chunk: index, url, bytes }));
        entry.loaded = index + 1;
        if let Some(init) = init {
            entry.inits.insert(init);
        }
        if !entry.played {
            self.held_bytes += bytes;
        }
        self.chunk_estimate = self.chunk_estimate.max(bytes);
        let mut evicted = Vec::new();
        let mut self_evicted = false;
        while self.held_bytes > budget {
            let Some((farthest, _)) = self.farthest(None) else { break };
            self_evicted |= farthest == key;
            evicted.push(self.evict_entry(&farthest));
        }
        if self_evicted {
            self.block(key);
        }
        (evicted, self_evicted)
    }

    fn block(&mut self, key: &str) {
        let epoch = self.epoch;
        if let Some(entry) = self.entries.get_mut(key) {
            entry.blocked_at = Some(epoch);
        }
    }

    fn farthest(&self, except: Option<&str>) -> Option<(String, usize)> {
        self.entries
            .iter()
            .filter(|(key, entry)| Some(key.as_str()) != except && entry.held_bytes() > 0)
            .max_by_key(|(key, entry)| (entry.distance, std::cmp::Reverse(entry.stamp), key.as_str()))
            .map(|(key, entry)| (key.clone(), entry.distance))
    }

    fn evict_entry(&mut self, key: &str) -> Evicted {
        let entry = self.entries.get_mut(key).expect("evicted entry exists");
        self.held_bytes -= entry.held_bytes();
        let urls = entry.held.drain(..).map(|held| held.url).collect();
        let headers = entry.plan.as_ref().map(|plan| plan.headers.clone()).unwrap_or_default();
        entry.inits.clear();
        entry.loaded = 0;
        self.epoch += 1;
        Evicted { urls, headers }
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
        if had_queued || had_active {
            self.counters.cancelled += 1;
        }
        had_queued
    }

    fn trim(&mut self, now: Duration) {
        if self.entries.len() <= self.max_entries {
            return;
        }
        let window = self.dedupe_window;
        self.entries.retain(|_, entry| entry.held_bytes() > 0 || now.saturating_sub(entry.stamp) <= window);
        while self.entries.len() > self.max_entries {
            let Some(oldest) = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.held_bytes() == 0)
                .min_by_key(|(key, entry)| (entry.stamp, key.as_str()))
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }
}
