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
    pub distance: usize,
}

pub fn relieve(pressure: Pressure, slots_state: &[SlotState]) -> Vec<Command> {
    let farthest_first = |retention: Retention| {
        let mut slots: Vec<&SlotState> =
            slots_state.iter().filter(|state| state.retention == retention && !state.is_playing).collect();
        slots.sort_by_key(|state| std::cmp::Reverse(state.distance));
        slots
    };
    let warm = farthest_first(Retention::Warm);
    let (hot, playing) = match pressure {
        Pressure::Moderate => (Vec::new(), Vec::new()),
        Pressure::Critical => (
            farthest_first(Retention::Hot),
            slots_state.iter().filter(|state| state.is_playing && state.retention != Retention::Cold).collect(),
        ),
    };
    warm.into_iter().chain(hot).chain(playing).map(|state| Command::Unload { slot: state.slot }).collect()
}
