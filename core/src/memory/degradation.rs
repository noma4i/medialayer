use crate::pool::Pressure;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Step {
    Normal,
    LowerQuality,
    ShorterBuffer,
    FewerSlots,
    PlayingSlotOnly,
    PlaceholdersOnly,
}

const STEPS: [Step; 6] =
    [Step::Normal, Step::LowerQuality, Step::ShorterBuffer, Step::FewerSlots, Step::PlayingSlotOnly, Step::PlaceholdersOnly];

impl Step {
    fn lowered(self, by: usize) -> Step {
        STEPS[(self as usize + by).min(STEPS.len() - 1)]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepPlan {
    pub bitrate_cap_kbps: Option<u32>,
    pub buffer_percent: u8,
    pub slot_percent: u8,
    pub min_slots: usize,
    pub reach_percent: u8,
    pub admission_percent: u8,
}

impl StepPlan {
    pub fn for_step(step: Step) -> Self {
        let plan = |bitrate_cap_kbps, buffer_percent, slot_percent, min_slots, reach_percent, admission_percent| Self {
            bitrate_cap_kbps,
            buffer_percent,
            slot_percent,
            min_slots,
            reach_percent,
            admission_percent,
        };
        match step {
            Step::Normal => plan(None, 100, 100, 0, 100, 100),
            Step::LowerQuality => plan(Some(2_500), 100, 100, 0, 100, 95),
            Step::ShorterBuffer => plan(Some(1_500), 50, 100, 0, 100, 90),
            Step::FewerSlots => plan(Some(1_000), 50, 50, 2, 50, 80),
            Step::PlayingSlotOnly => plan(Some(700), 25, 0, 1, 0, 65),
            Step::PlaceholdersOnly => plan(Some(0), 0, 0, 0, 0, 40),
        }
    }

    pub fn buffer_seconds(&self, base: u32) -> u32 {
        if self.buffer_percent == 0 {
            return 0;
        }
        (base * u32::from(self.buffer_percent) / 100).max(1).min(base)
    }

    pub fn slot_limit(&self, slots: usize) -> usize {
        (slots * usize::from(self.slot_percent) / 100).max(self.min_slots).min(slots)
    }

    pub fn reach_limit(&self, reach: usize) -> usize {
        reach * usize::from(self.reach_percent) / 100
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DegradationTuning {
    pub enter_percent: [u8; 5],
    pub margin_percent: u8,
}

impl Default for DegradationTuning {
    fn default() -> Self {
        Self { enter_percent: [70, 80, 88, 94, 99], margin_percent: 5 }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Degradation {
    tuning: DegradationTuning,
    held: Step,
}

impl Default for Degradation {
    fn default() -> Self {
        Self::new(DegradationTuning::default())
    }
}

impl Degradation {
    pub fn new(tuning: DegradationTuning) -> Self {
        Self { tuning, held: Step::Normal }
    }

    pub fn for_pressure(&mut self, used: u64, ceiling: u64, system: Option<Pressure>) -> Step {
        self.held = self.load_step(used, ceiling);
        let shift = match system {
            None => 0,
            Some(Pressure::Moderate) => 1,
            Some(Pressure::Critical) => 2,
        };
        self.held.lowered(shift)
    }

    fn load_step(&self, used: u64, ceiling: u64) -> Step {
        if ceiling == 0 {
            return Step::PlaceholdersOnly;
        }
        let percent = u128::from(used) * 100 / u128::from(ceiling);
        let reached = |margin: u8| {
            self.tuning
                .enter_percent
                .iter()
                .take_while(|enter| percent >= u128::from(enter.saturating_sub(margin)))
                .count()
        };
        let target = STEPS[reached(0)];
        if target >= self.held {
            return target;
        }
        STEPS[reached(self.tuning.margin_percent)].min(self.held).max(target)
    }
}
