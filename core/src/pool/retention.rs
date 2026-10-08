use super::ring::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    Hot,
    Warm,
    Cold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    Moderate,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotState {
    pub slot: usize,
    pub retention: Retention,
    pub is_playing: bool,
    pub is_pinned: bool,
    pub distance: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    StopPrefetch,
    DropMemoryCache,
    Slot(Command),
}

pub fn relieve(pressure: Pressure, slots_state: &[SlotState]) -> Vec<Directive> {
    let farthest_first = |retention: Retention| {
        let mut slots: Vec<&SlotState> = slots_state
            .iter()
            .filter(|state| state.retention == retention && !state.is_playing && !state.is_pinned)
            .collect();
        slots.sort_by_key(|state| std::cmp::Reverse(state.distance));
        slots
    };
    let unloads = |states: Vec<&SlotState>| -> Vec<Directive> {
        states.into_iter().map(|state| Directive::Slot(Command::Unload { slot: state.slot })).collect()
    };
    let warm = unloads(farthest_first(Retention::Warm));
    if pressure == Pressure::Moderate {
        return [vec![Directive::StopPrefetch], warm].concat();
    }
    let hot = unloads(farthest_first(Retention::Hot));
    let playing = unloads(
        slots_state.iter().filter(|state| state.is_playing && !state.is_pinned && state.retention != Retention::Cold).collect(),
    );
    let pinned = unloads(slots_state.iter().filter(|state| state.is_pinned && state.retention != Retention::Cold).collect());
    [vec![Directive::StopPrefetch], warm, hot, vec![Directive::DropMemoryCache], playing, pinned].concat()
}
