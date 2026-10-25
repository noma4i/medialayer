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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    Load { slot: usize, key: String },
    Unload { slot: usize },
    Dispose { slot: usize },
    Preroll { slot: usize },
    Play { slot: usize },
    Pause { slot: usize },
    Mute { slot: usize },
    Unmute { slot: usize },
    Retain { slot: usize, level: Retention },
}

impl From<&Command> for Shape {
    fn from(command: &Command) -> Shape {
        match command {
            Command::Load { slot, key, .. } => Shape::Load { slot: *slot, key: key.clone() },
            Command::Unload { slot } => Shape::Unload { slot: *slot },
            Command::Dispose { slot } => Shape::Dispose { slot: *slot },
            Command::Preroll { slot } => Shape::Preroll { slot: *slot },
            Command::Play { slot } => Shape::Play { slot: *slot },
            Command::Pause { slot } => Shape::Pause { slot: *slot },
            Command::Mute { slot } => Shape::Mute { slot: *slot },
            Command::Unmute { slot } => Shape::Unmute { slot: *slot },
            Command::Retain { slot, level } => Shape::Retain { slot: *slot, level: *level },
        }
    }
}

impl PartialEq<Shape> for Command {
    fn eq(&self, other: &Shape) -> bool {
        Shape::from(self) == *other
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    StopPrefetch,
    DropMemoryCache,
    Slot(Shape),
}

pub fn plan_of(directives: &[Directive]) -> Vec<Plan> {
    directives
        .iter()
        .map(|directive| match directive {
            Directive::StopPrefetch => Plan::StopPrefetch,
            Directive::DropMemoryCache => Plan::DropMemoryCache,
            Directive::Slot(command) => Plan::Slot(Shape::from(command)),
        })
        .collect()
}

pub fn load(slot: usize, key: &str) -> Shape {
    Shape::Load { slot, key: key.to_string() }
}

pub fn slot_of(command: &Command) -> usize {
    match command {
        Command::Load { slot, .. }
        | Command::Unload { slot }
        | Command::Dispose { slot }
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
