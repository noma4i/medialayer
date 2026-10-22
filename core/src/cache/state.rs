use super::entry::{Entry, Kind, KindStats, CacheStats};
use super::verify::Verified;
use crate::analysis::ToneData;
use crate::limits::Limits;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Bound;
use std::path::PathBuf;
use std::sync::Arc;

pub struct Slot {
    pub version: u64,
    pub key: String,
    pub entry: Entry,
    pub readers: u32,
    pub verified: Arc<Verified>,
    pub kept: bool,
}

pub struct Item {
    pub name: String,
    pub key: String,
    pub entry: Entry,
}

struct Writing {
    count: u32,
    generation: u64,
    key: String,
    kind: Kind,
}

pub struct State {
    pub changes: BTreeMap<String, u64>,
    pub revision: u64,
    pub order: super::policy::EvictionOrder,
    #[cfg(test)]
    pub keep_visits: usize,
    pub limits: Limits,
    pub used: u64,
    pub reserved: u64,
    pub retained_bytes: u64,
    pub evictions: u64,
    pub live_parts: HashSet<PathBuf>,
    entries: BTreeMap<String, Slot>,
    keys: BTreeMap<String, String>,
    version: u64,
    retired: HashMap<u64, (u64, u32)>,
    per_kind: [KindStats; 3],
    writing: HashMap<String, Writing>,
}

impl State {
    pub fn new(limits: Limits) -> State {
        State {
            changes: BTreeMap::new(),
            revision: 0,
            order: super::policy::EvictionOrder::new(limits.strategy, &HashMap::new()),
            #[cfg(test)]
            keep_visits: 0,
            limits,
            used: 0,
            reserved: 0,
            retained_bytes: 0,
            evictions: 0,
            live_parts: HashSet::new(),
            entries: BTreeMap::new(),
            keys: BTreeMap::new(),
            version: 0,
            retired: HashMap::new(),
            per_kind: [KindStats::default(); 3],
            writing: HashMap::new(),
        }
    }

    pub fn keep(&mut self, key: &str) {
        let prefix = format!("{key}#");
        let names = self.keys.get(key).into_iter().chain(
            self.keys.range(prefix.clone()..).take_while(|(key, _)| key.starts_with(&prefix)).map(|(_, name)| name),
        );
        for name in names {
            #[cfg(test)]
            { self.keep_visits += 1; }
            if let Some(slot) = self.entries.get_mut(name) {
                self.order.remove(name, slot);
                slot.kept = true;
                if slot.readers == 0 && !self.writing.contains_key(name) {
                    self.order.insert(name, slot);
                }
            }
        }
    }

    #[cfg(test)]
    pub fn is_kept(&self, key: &str) -> bool {
        self.entries.get(&super::naming::file_name(key)).is_some_and(|slot| slot.kept)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    pub fn slot(&self, name: &str) -> Option<&Slot> {
        self.entries.get(name)
    }

    pub fn set_tone(&mut self, name: &str, tone: ToneData) -> bool {
        let changed = self.entries.get_mut(name).map(|slot| slot.entry.tone = Some(tone)).is_some();
        if changed {
            self.changed(name);
        }
        changed
    }

    fn changed(&mut self, name: &str) {
        self.revision += 1;
        self.changes.insert(name.to_owned(), self.revision);
    }

    pub fn insert(&mut self, name: String, key: String, entry: Entry, written_here: bool) {
        let kept = self.take(&name).is_some_and(|slot| slot.kept);
        self.changed(&name);
        self.version += 1;
        self.used += entry.size;
        let stats = &mut self.per_kind[entry.kind.index()];
        stats.bytes += entry.size;
        stats.files += 1;
        self.keys.insert(key.clone(), name.clone());
        let verified = if written_here { Verified::written(&entry.check) } else { Verified::loaded(&entry.check) };
        let slot = Slot { key, entry, readers: 0, verified, kept, version: self.version };
        if !self.writing.contains_key(&name) {
            self.order.insert(&name, &slot);
        }
        self.entries.insert(name, slot);
    }

    pub fn take(&mut self, name: &str) -> Option<Slot> {
        let slot = self.entries.remove(name)?;
        self.changed(name);
        self.order.remove(name, &slot);
        if slot.readers > 0 {
            self.retained_bytes += slot.entry.size;
            self.retired.insert(slot.version, (slot.entry.size, slot.readers));
        }
        self.keys.remove(&slot.key);
        self.used -= slot.entry.size;
        let stats = &mut self.per_kind[slot.entry.kind.index()];
        stats.bytes -= slot.entry.size;
        stats.files -= 1;
        Some(slot)
    }

    pub fn pin(&mut self, name: &str, now: u64) -> Option<(u64, u64, Arc<Verified>)> {
        let slot = self.entries.get_mut(name)?;
        self.order.remove(name, slot);
        slot.readers += 1;
        slot.entry.last_access = now;
        slot.entry.hits += 1;
        let result = (slot.entry.size, slot.version, slot.verified.clone());
        self.changed(name);
        Some(result)
    }

    pub fn unpin(&mut self, name: &str, version: u64) {
        if let Some(slot) = self.entries.get_mut(name).filter(|slot| slot.version == version) {
            slot.readers = slot.readers.saturating_sub(1);
            if slot.readers == 0 && !self.writing.contains_key(name) {
                self.order.insert(name, slot);
            }
        } else if let Some((size, readers)) = self.retired.get_mut(&version) {
            *readers -= 1;
            if *readers == 0 {
                self.retained_bytes -= *size;
                self.retired.remove(&version);
            }
        }
    }

    pub fn start_writing(&mut self, name: &str, key: &str, kind: Kind) -> u64 {
        if let Some(slot) = self.entries.get(name) {
            self.order.remove(name, slot);
        }
        let writing = self.writing.entry(name.to_string()).or_insert_with(|| Writing { count: 0, generation: 0, key: key.to_owned(), kind });
        writing.count += 1;
        writing.generation
    }

    pub fn stop_writing(&mut self, name: &str) {
        if let Some(writing) = self.writing.get_mut(name) {
            writing.count -= 1;
            if writing.count == 0 {
                self.writing.remove(name);
                if let Some(slot) = self.entries.get(name).filter(|slot| slot.readers == 0) {
                    self.order.insert(name, slot);
                }
            }
        }
    }

    pub fn is_protected(&self, name: &str) -> bool {
        self.writing.contains_key(name) || self.entries.get(name).is_some_and(|slot| slot.readers > 0)
    }

    pub fn invalidate_writers(&mut self, wanted: impl Fn(&str, Kind) -> bool) -> Vec<String> {
        let mut names = Vec::new();
        for (name, writing) in &mut self.writing {
            if wanted(&writing.key, writing.kind) {
                writing.generation += 1;
                names.push(name.clone());
            }
        }
        names
    }

    pub fn writer_is_current(&self, name: &str, generation: u64) -> bool {
        self.writing.get(name).is_some_and(|writing| writing.generation == generation)
    }

    pub fn configure_order(&mut self, overrides: &HashMap<Kind, crate::limits::EvictionStrategy>) {
        let mut order = super::policy::EvictionOrder::new(self.limits.strategy, overrides);
        for (name, slot) in &self.entries {
            if !self.is_protected(name) {
                order.insert(name, slot);
            }
        }
        self.order = order;
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
            open_writers: self.writing.values().map(|writing| u64::from(writing.count)).sum(),
            eviction_backlog: 0,
        }
    }
}
