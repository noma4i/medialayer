use std::collections::{HashMap, VecDeque};

const CAPACITY: usize = 32;

#[derive(Default)]
pub(super) struct ResumePositions {
    order: VecDeque<String>,
    at: HashMap<String, u64>,
}

impl ResumePositions {
    pub(super) fn remember(&mut self, key: &str, position_ms: u64) {
        if self.at.insert(key.to_string(), position_ms).is_none() {
            self.order.push_back(key.to_string());
        }
        while self.order.len() > CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.at.remove(&oldest);
            }
        }
    }

    pub(super) fn take(&mut self, key: &str) -> Option<u64> {
        let position = self.at.remove(key)?;
        self.order.retain(|queued| queued != key);
        Some(position)
    }

    pub(super) fn clear(&mut self) {
        self.order.clear();
        self.at.clear();
    }
}
