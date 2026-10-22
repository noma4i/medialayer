use std::collections::{HashMap, VecDeque};

const CAPACITY: usize = 32;

#[derive(Clone, Copy)]
enum Pending {
    Millis(u64),
    Fraction(f64),
}

#[derive(Default)]
pub(super) struct ResumePositions {
    order: VecDeque<String>,
    at: HashMap<String, Pending>,
}

impl ResumePositions {
    pub(super) fn remember(&mut self, key: &str, position_ms: u64) {
        self.insert(key, Pending::Millis(position_ms));
    }

    pub(super) fn remember_fraction(&mut self, key: &str, fraction: f64) {
        self.insert(key, Pending::Fraction(fraction));
    }

    pub(super) fn take(&mut self, key: &str) -> Option<u64> {
        let Some(Pending::Millis(position)) = self.at.get(key).copied() else { return None };
        self.forget(key);
        Some(position)
    }

    pub(super) fn take_fraction(&mut self, key: &str) -> Option<f64> {
        let Some(Pending::Fraction(fraction)) = self.at.get(key).copied() else { return None };
        self.forget(key);
        Some(fraction)
    }

    pub(super) fn forget(&mut self, key: &str) {
        if self.at.remove(key).is_some() {
            self.order.retain(|queued| queued != key);
        }
    }

    pub(super) fn clear(&mut self) {
        self.order.clear();
        self.at.clear();
    }

    fn insert(&mut self, key: &str, pending: Pending) {
        if self.at.insert(key.to_string(), pending).is_none() {
            self.order.push_back(key.to_string());
        }
        while self.order.len() > CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.at.remove(&oldest);
            }
        }
    }
}
