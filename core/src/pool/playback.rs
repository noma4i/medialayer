use super::ring::{Command, Ring};

impl Ring {
    pub fn set_playing(&mut self, key: &str, playing: bool) -> Vec<Command> {
        let Some(target) = self.slot_of(key) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if playing {
            for slot in 0..self.slots {
                if slot == target {
                    continue;
                }
                if let Some(binding) = self.bindings[slot].as_mut().filter(|binding| binding.is_playing) {
                    binding.is_playing = false;
                    out.push(Command::Pause { slot });
                }
            }
        }
        if let Some(binding) = self.bindings[target].as_mut().filter(|binding| binding.is_playing != playing) {
            binding.is_playing = playing;
            out.push(if playing { Command::Play { slot: target } } else { Command::Pause { slot: target } });
        }
        out
    }

    pub fn handoff(&mut self, from: &str, to: &str) -> Vec<Command> {
        let (Some(source), Some(target)) = (self.slot_of(from), self.slot_of(to)) else {
            return Vec::new();
        };
        if source == target || !self.bindings[source].as_ref().is_some_and(|binding| binding.is_playing) {
            return Vec::new();
        }
        let mut out = Vec::new();
        if let Some(binding) = self.bindings[target].as_mut().filter(|binding| !binding.is_playing) {
            binding.is_playing = true;
            out.push(Command::Play { slot: target });
        }
        if let Some(binding) = self.bindings[source].as_mut() {
            binding.is_playing = false;
        }
        out.push(Command::Pause { slot: source });
        out
    }

    pub fn set_muted_all(&mut self, muted: bool) -> Vec<Command> {
        self.muted_all = muted;
        self.sync_mute()
    }

    pub fn set_muted(&mut self, key: &str, muted: bool) -> Vec<Command> {
        if muted {
            self.muted_keys.insert(key.to_string());
        } else {
            self.muted_keys.remove(key);
        }
        self.sync_mute()
    }

    pub fn forget_mute(&mut self, key: &str) {
        self.muted_keys.remove(key);
    }

    pub fn muted_key_count(&self) -> usize {
        self.muted_keys.len()
    }

    pub(super) fn sync_mute(&mut self) -> Vec<Command> {
        let mut out = Vec::new();
        for slot in 0..self.slots {
            let Some(binding) = self.bindings[slot].as_ref() else {
                continue;
            };
            let muted = self.muted_all || self.muted_keys.contains(&binding.key);
            if muted == self.native_muted[slot] {
                continue;
            }
            self.native_muted[slot] = muted;
            out.push(if muted { Command::Mute { slot } } else { Command::Unmute { slot } });
        }
        out
    }
}
