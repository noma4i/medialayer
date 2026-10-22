use std::collections::{BTreeMap, HashMap};

pub(super) struct Recent<V> {
    limit: usize,
    tick: u64,
    items: HashMap<String, (u64, V)>,
    order: BTreeMap<u64, String>,
}

impl<V> Recent<V> {
    pub fn new(limit: usize) -> Self {
        Recent { limit, tick: 0, items: HashMap::new(), order: BTreeMap::new() }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    fn restamp(&mut self, key: &str) -> Option<&mut V> {
        self.tick += 1;
        let (stamp, value) = self.items.get_mut(key)?;
        let moved = self.order.remove(stamp)?;
        *stamp = self.tick;
        self.order.insert(self.tick, moved);
        Some(value)
    }

    pub fn get(&mut self, key: &str) -> Option<&V> {
        self.restamp(key).map(|value| &*value)
    }

    pub fn insert(&mut self, key: &str, value: V, keep: impl Fn(&str, &V) -> bool) {
        if let Some(existing) = self.restamp(key) {
            *existing = value;
            return;
        }
        while self.items.len() >= self.limit {
            let victim = self.order.iter().find(|(_, name)| self.items.get(*name).is_some_and(|(_, value)| !keep(name, value))).map(|(stamp, _)| *stamp);
            let Some(stamp) = victim else { break };
            self.remove_stamped(stamp);
        }
        self.tick += 1;
        self.order.insert(self.tick, key.to_string());
        self.items.insert(key.to_string(), (self.tick, value));
    }

    fn remove_stamped(&mut self, stamp: u64) -> Option<(String, V)> {
        let name = self.order.remove(&stamp)?;
        self.items.remove(&name).map(|(_, value)| (name, value))
    }

    pub fn pop_oldest(&mut self) -> Option<(String, V)> {
        let stamp = *self.order.keys().next()?;
        self.remove_stamped(stamp)
    }

    pub fn remove(&mut self, key: &str) -> Option<V> {
        let (stamp, value) = self.items.remove(key)?;
        self.order.remove(&stamp);
        Some(value)
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&str, &V) -> bool) {
        let order = &mut self.order;
        self.items.retain(|name, (stamp, value)| {
            let kept = keep(name, value);
            if !kept {
                order.remove(stamp);
            }
            kept
        });
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.order.clear();
    }
}
