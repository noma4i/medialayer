use super::entry::{Entry, Kind, KindStats, CacheStats};
use crate::limits::Limits;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Bound;
use std::path::PathBuf;

pub struct Slot {
    pub key: String,
    pub entry: Entry,
    pub readers: u32,
}

pub struct Item {
    pub name: String,
    pub key: String,
    pub entry: Entry,
    pub protected: bool,
}

pub struct State {
    pub limits: Limits,
    pub used: u64,
    pub reserved: u64,
    pub evictions: u64,
    pub live_parts: HashSet<PathBuf>,
    entries: BTreeMap<String, Slot>,
    per_kind: [KindStats; 3],
    writing: HashMap<String, u32>,
}

impl State {
    pub fn new(limits: Limits) -> State {
        State {
            limits,
            used: 0,
            reserved: 0,
            evictions: 0,
            live_parts: HashSet::new(),
            entries: BTreeMap::new(),
            per_kind: [KindStats::default(); 3],
            writing: HashMap::new(),
        }
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    pub fn entry(&self, name: &str) -> Option<Entry> {
        self.entries.get(name).map(|slot| slot.entry.clone())
    }

    pub fn insert(&mut self, name: String, key: String, entry: Entry) {
        let readers = self.take(&name).map_or(0, |slot| slot.readers);
        self.used += entry.size;
        let stats = &mut self.per_kind[entry.kind.index()];
        stats.bytes += entry.size;
        stats.files += 1;
        self.entries.insert(name, Slot { key, entry, readers });
    }

    pub fn take(&mut self, name: &str) -> Option<Slot> {
        let slot = self.entries.remove(name)?;
        self.used -= slot.entry.size;
        let stats = &mut self.per_kind[slot.entry.kind.index()];
        stats.bytes -= slot.entry.size;
        stats.files -= 1;
        Some(slot)
    }

    pub fn pin(&mut self, name: &str, now: u64) -> Option<u64> {
        let slot = self.entries.get_mut(name)?;
        slot.readers += 1;
        slot.entry.last_access = now;
        slot.entry.hits += 1;
        Some(slot.entry.size)
    }

    pub fn unpin(&mut self, name: &str) {
        if let Some(slot) = self.entries.get_mut(name) {
            slot.readers = slot.readers.saturating_sub(1);
        }
    }

    pub fn start_writing(&mut self, name: &str) {
        *self.writing.entry(name.to_string()).or_insert(0) += 1;
    }

    pub fn stop_writing(&mut self, name: &str) {
        if let Some(count) = self.writing.get_mut(name) {
            *count -= 1;
            if *count == 0 {
                self.writing.remove(name);
            }
        }
    }

    pub fn is_protected(&self, name: &str) -> bool {
        self.writing.contains_key(name) || self.entries.get(name).is_some_and(|slot| slot.readers > 0)
    }

    pub fn chunk(&self, after: Option<&str>, limit: usize) -> Vec<Item> {
        let lower = after.map_or(Bound::Unbounded, Bound::Excluded);
        self.entries
            .range::<str, _>((lower, Bound::Unbounded))
            .take(limit)
            .map(|(name, slot)| Item {
                name: name.clone(),
                key: slot.key.clone(),
                entry: slot.entry.clone(),
                protected: self.is_protected(name),
            })
            .collect()
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            total_bytes: self.used,
            file_count: self.entries.len() as u64,
            per_kind: Kind::ALL.into_iter().map(|kind| (kind, self.per_kind[kind.index()])).collect(),
            evictions: self.evictions,
            max_hold: std::time::Duration::ZERO,
            open_writers: self.writing.values().map(|count| u64::from(*count)).sum(),
            eviction_backlog: 0,
        }
    }
}
