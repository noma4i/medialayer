use super::*;

pub fn keys(count: usize) -> Vec<Option<String>> {
    (0..count).map(|index| Some(format!("k{index}"))).collect()
}

pub fn ring(slots: usize, reach: usize) -> Ring {
    Ring::new(PoolConfig::new(slots, reach))
}

pub fn ring_with(slots: usize, reach: usize, adjust: impl FnOnce(&mut PoolConfig)) -> Ring {
    let mut config = PoolConfig::new(slots, reach);
    adjust(&mut config);
    Ring::new(config)
}

pub fn slot_commands(directives: &[Directive]) -> Vec<Command> {
    directives
        .iter()
        .filter_map(|directive| match directive {
            Directive::Slot(command) => Some(command.clone()),
            _ => None,
        })
        .collect()
}

pub fn slot_of(command: &Command) -> usize {
    match command {
        Command::Load { slot, .. }
        | Command::Unload { slot }
        | Command::Preroll { slot }
        | Command::Play { slot }
        | Command::Pause { slot }
        | Command::Mute { slot }
        | Command::Unmute { slot }
        | Command::Retain { slot, .. } => *slot,
    }
}

pub fn walk(count: usize, steps: usize) -> Vec<usize> {
    let mut dice = crate::seeded::Dice::new("pool test_support walk");
    let mut active = 0usize;
    (0..steps)
        .map(|_| {
            let state = dice.next_u64();
            let jump = (state >> 33) % 5;
            active = match jump {
                0 => active.saturating_sub(1),
                1 | 2 => (active + 1).min(count - 1),
                3 => (state >> 40) as usize % count,
                _ => active,
            };
            active
        })
        .collect()
}
