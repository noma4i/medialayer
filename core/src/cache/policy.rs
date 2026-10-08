use super::entry::{Entry, Kind};
use crate::limits::EvictionStrategy;
use std::cmp::Ordering;
use std::collections::HashMap;

const MS_PER_SECOND: u64 = 1000;

struct Group {
    strategy: EvictionStrategy,
    priority: usize,
    members: Vec<(String, Entry)>,
}

pub fn victim_order(
    candidates: Vec<(String, Entry)>,
    default: EvictionStrategy,
    overrides: &HashMap<Kind, EvictionStrategy>,
    kind_priority: &[Kind; 3],
    now: u64,
) -> Vec<String> {
    let mut groups: Vec<Group> = Vec::new();
    for candidate in candidates {
        let strategy = overrides.get(&candidate.1.kind).copied().unwrap_or(default);
        let priority = kind_priority.iter().position(|kind| *kind == candidate.1.kind).unwrap_or(kind_priority.len());
        match groups.iter_mut().find(|group| group.strategy == strategy) {
            Some(group) => {
                group.priority = group.priority.min(priority);
                group.members.push(candidate);
            }
            None => groups.push(Group { strategy, priority, members: vec![candidate] }),
        }
    }
    groups.sort_by_key(|group| group.priority);
    groups
        .into_iter()
        .flat_map(|mut group| {
            group.members.sort_by(|left, right| compare(group.strategy, now, left, right));
            group.members.into_iter().map(|(name, _)| name)
        })
        .collect()
}

fn compare(strategy: EvictionStrategy, now: u64, left: &(String, Entry), right: &(String, Entry)) -> Ordering {
    rank(strategy, now, &left.1).cmp(&rank(strategy, now, &right.1)).then_with(|| left.0.cmp(&right.0))
}

fn rank(strategy: EvictionStrategy, now: u64, entry: &Entry) -> [u64; 2] {
    match strategy {
        EvictionStrategy::Lru => [entry.last_access, 0],
        EvictionStrategy::Fifo => [entry.created, 0],
        EvictionStrategy::Lfu => [entry.hits, entry.last_access],
        EvictionStrategy::LargestFirst => [u64::MAX - entry.size, 0],
        EvictionStrategy::ExpireAfter(seconds) => {
            let expired = now >= entry.created.saturating_add(u64::from(seconds) * MS_PER_SECOND);
            [u64::from(!expired), entry.created]
        }
    }
}
