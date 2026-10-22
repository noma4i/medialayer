use super::entry::{Entry, Kind};
use crate::limits::EvictionStrategy;
use super::state::Slot;
use std::collections::{BTreeSet, HashMap, HashSet};

type Ranked = (bool, [u64; 2], String, u64);

pub struct EvictionOrder {
    strategies: [EvictionStrategy; 3],
    entries: [BTreeSet<Ranked>; 3],
}

impl EvictionOrder {
    pub fn new(default: EvictionStrategy, overrides: &HashMap<Kind, EvictionStrategy>) -> Self {
        Self {
            strategies: Kind::ALL.map(|kind| overrides.get(&kind).copied().unwrap_or(default)),
            entries: std::array::from_fn(|_| BTreeSet::new()),
        }
    }

    fn ranked(&self, name: &str, slot: &Slot) -> Ranked {
        (slot.kept, rank(self.strategies[slot.entry.kind.index()], &slot.entry), name.to_owned(), slot.version)
    }

    pub fn insert(&mut self, name: &str, slot: &Slot) {
        let ranked = self.ranked(name, slot);
        self.entries[slot.entry.kind.index()].insert(ranked);
    }

    pub fn remove(&mut self, name: &str, slot: &Slot) {
        let ranked = self.ranked(name, slot);
        self.entries[slot.entry.kind.index()].remove(&ranked);
    }

    pub fn candidates(&self, priority: &[Kind; 3], limit: usize, excluded: &HashSet<(String, u64)>) -> Vec<(String, u64)> {
        let mut result = Vec::new();
        for kept in [false, true] {
            let lower = (kept, [0, 0], String::new(), 0);
            let mut iterators = self.entries.each_ref().map(|entries| entries.range(lower.clone()..).take_while(|entry| entry.0 == kept).peekable());
            let priorities = Kind::ALL.map(|kind| priority.iter().position(|other| *other == kind).unwrap_or(priority.len()));
            let group_priorities = Kind::ALL.map(|kind| {
                Kind::ALL.into_iter().filter(|other| self.strategies[other.index()] == self.strategies[kind.index()] && iterators[other.index()].peek().is_some())
                    .map(|other| priorities[other.index()]).min().unwrap_or(priority.len())
            });
            while result.len() < limit {
                let selected = Kind::ALL.into_iter().filter_map(|kind| {
                    iterators[kind.index()].peek().map(|entry| (group_priorities[kind.index()], entry.1, entry.2.clone(), kind.index()))
                }).min();
                let Some((_, _, _, index)) = selected else { break };
                let Some(entry) = iterators[index].next() else { break };
                let candidate = (entry.2.clone(), entry.3);
                if !excluded.contains(&candidate) {
                    result.push(candidate);
                }
            }
        }
        result
    }
}

fn rank(strategy: EvictionStrategy, entry: &Entry) -> [u64; 2] {
    match strategy {
        EvictionStrategy::Lru => [entry.last_access, 0],
        EvictionStrategy::Fifo | EvictionStrategy::ExpireAfter(_) => [entry.created, 0],
        EvictionStrategy::Lfu => [entry.hits, entry.last_access],
        EvictionStrategy::LargestFirst => [u64::MAX - entry.size, 0],
    }
}
