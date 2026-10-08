use super::retention::{Pressure, Retention, SlotState, relieve};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Load { slot: usize, key: String },
    Unload { slot: usize },
    Preroll { slot: usize },
    Play { slot: usize },
    Pause { slot: usize },
}

#[derive(Debug, Clone)]
struct Binding {
    page: usize,
    key: Option<String>,
    is_playing: bool,
    is_prerolled: bool,
}

#[derive(Debug, Clone)]
pub struct Ring {
    slots: usize,
    reach: usize,
    bindings: Vec<Option<Binding>>,
}

impl Ring {
    pub fn new(slots: usize, reach: usize) -> Self {
        let slots = slots.max(2 * reach + 1);
        Self { slots, reach, bindings: vec![None; slots] }
    }

    pub fn slots(&self) -> usize {
        self.slots
    }

    pub fn slot(&self, page: usize) -> usize {
        page % self.slots
    }

    pub fn pages(&self, active: usize, count: usize) -> Range<usize> {
        if count == 0 {
            return 0..0;
        }
        active.saturating_sub(self.reach)..active.saturating_add(self.reach).saturating_add(1).min(count)
    }

    pub fn plan(&mut self, active: usize, playback: Option<usize>, keys: &[Option<String>]) -> Vec<Command> {
        let wanted = self.pages(active, keys.len());
        let playback = playback.filter(|page| wanted.contains(page));
        let mut unloads = Vec::new();
        let mut loads = Vec::new();

        for slot in 0..self.slots {
            let outside = self.bindings[slot].as_ref().is_some_and(|binding| !wanted.contains(&binding.page));
            if outside {
                self.release(slot, &mut unloads);
            }
        }
        for page in wanted.clone() {
            let slot = self.slot(page);
            let key = keys[page].clone();
            let is_current = self.bindings[slot].as_ref().is_some_and(|binding| binding.page == page && binding.key == key);
            if is_current {
                continue;
            }
            self.release(slot, &mut unloads);
            if let Some(key) = &key {
                loads.push(Command::Load { slot, key: key.clone() });
            }
            self.bindings[slot] = Some(Binding { page, key, is_playing: false, is_prerolled: false });
        }

        let mut pauses = Vec::new();
        let mut plays = Vec::new();
        let mut prerolls = Vec::new();
        for slot in 0..self.slots {
            let Some(binding) = self.bindings[slot].as_mut().filter(|binding| binding.key.is_some()) else {
                continue;
            };
            let should_play = playback == Some(binding.page);
            if binding.is_playing && !should_play {
                binding.is_playing = false;
                pauses.push(Command::Pause { slot });
            }
            if !binding.is_playing && should_play {
                binding.is_playing = true;
                plays.push(Command::Play { slot });
            }
            if !should_play && !binding.is_prerolled {
                binding.is_prerolled = true;
                prerolls.push(Command::Preroll { slot });
            }
        }
        [unloads, loads, pauses, plays, prerolls].concat()
    }

    pub fn states(&self, active: usize) -> Vec<SlotState> {
        self.bindings
            .iter()
            .enumerate()
            .map(|(slot, binding)| match binding.as_ref().filter(|binding| binding.key.is_some()) {
                None => SlotState { slot, retention: Retention::Cold, is_playing: false, distance: usize::MAX },
                Some(binding) => {
                    let is_hot = binding.is_playing || binding.page == active.saturating_add(1);
                    SlotState {
                        slot,
                        retention: if is_hot { Retention::Hot } else { Retention::Warm },
                        is_playing: binding.is_playing,
                        distance: binding.page.abs_diff(active),
                    }
                }
            })
            .collect()
    }

    pub fn release_under(&mut self, pressure: Pressure, active: usize) -> Vec<Command> {
        let commands = relieve(pressure, &self.states(active));
        for command in &commands {
            if let Command::Unload { slot } = command {
                self.bindings[*slot] = None;
            }
        }
        commands
    }

    fn release(&mut self, slot: usize, unloads: &mut Vec<Command>) {
        let Some(binding) = self.bindings[slot].take() else {
            return;
        };
        if binding.key.is_some() {
            unloads.push(Command::Unload { slot });
        }
    }
}
