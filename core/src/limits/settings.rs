use super::chunks::LeadingChunks;

pub(crate) const MIB: u64 = 1024 * 1024;
const MAX_PREFETCH_BUDGET_MIB: u32 = 512;
const GIB: u64 = 1024 * MIB;
pub const MAX_CHUNKS_AHEAD: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionStrategy {
    Lru,
    Fifo,
    Lfu,
    LargestFirst,
    ExpireAfter(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub budget: u64,
    pub min_budget: u64,
    pub shrink_to: f64,
    pub memory_budget: u64,
    pub slots: u32,
    pub prefetch_from: u32,
    pub prefetch_to: u32,
    pub leading_chunks: LeadingChunks,
    pub prefetch_budget_mib: u32,
    pub timeout_seconds: u32,
    pub max_concurrent: u32,
    pub dedupe_window_seconds: u32,
    pub dedupe_max_entries: u32,
    pub playback_chunks_ahead: u32,
    pub strategy: EvictionStrategy,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LimitsError {
    #[error("budget {budget} is below min_budget {min_budget}")]
    BudgetBelowMin { budget: u64, min_budget: u64 },
    #[error("shrink_to must be in (0, 1]")]
    ShrinkToOutOfRange,
    #[error("prefetch window from {from} is above to {to}")]
    PrefetchWindowInverted { from: u32, to: u32 },
    #[error("{field} must not be zero")]
    Zero { field: &'static str },
    #[error("leading_chunks {reason}")]
    LeadingChunks { reason: &'static str },
    #[error("playback_chunks_ahead must be between 1 and {MAX_CHUNKS_AHEAD}")]
    ChunksAhead,
    #[error("prefetch_budget_mib must be between 1 and {MAX_PREFETCH_BUDGET_MIB}")]
    PrefetchBudget,
    #[error("line {line} is not key=value")]
    MalformedLine { line: usize },
    #[error("unknown key {key}")]
    UnknownKey { key: String },
    #[error("invalid value for {key}")]
    InvalidValue { key: &'static str },
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            budget: 4 * GIB,
            min_budget: 64 * MIB,
            shrink_to: 0.9,
            memory_budget: 256 * MIB,
            slots: 3,
            prefetch_from: 1,
            prefetch_to: 3,
            leading_chunks: LeadingChunks::default(),
            prefetch_budget_mib: 64,
            timeout_seconds: 30,
            max_concurrent: 2,
            dedupe_window_seconds: 60,
            dedupe_max_entries: 500,
            playback_chunks_ahead: 3,
            strategy: EvictionStrategy::Lru,
        }
    }
}

impl Limits {
    pub fn validate(&self) -> Result<(), LimitsError> {
        let required = [
            ("budget", self.budget),
            ("memory_budget", self.memory_budget),
            ("slots", self.slots.into()),
            ("prefetch_budget_mib", self.prefetch_budget_mib.into()),
            ("timeout_seconds", self.timeout_seconds.into()),
            ("max_concurrent", self.max_concurrent.into()),
            ("dedupe_window_seconds", self.dedupe_window_seconds.into()),
            ("dedupe_max_entries", self.dedupe_max_entries.into()),
        ];
        if let Some((field, _)) = required.iter().find(|(_, value)| *value == 0) {
            return Err(LimitsError::Zero { field });
        }
        if self.prefetch_budget_mib > MAX_PREFETCH_BUDGET_MIB {
            return Err(LimitsError::PrefetchBudget);
        }
        if !(1..=MAX_CHUNKS_AHEAD).contains(&self.playback_chunks_ahead) {
            return Err(LimitsError::ChunksAhead);
        }
        if self.strategy == EvictionStrategy::ExpireAfter(0) {
            return Err(LimitsError::Zero { field: "strategy" });
        }
        if self.budget < self.min_budget {
            return Err(LimitsError::BudgetBelowMin { budget: self.budget, min_budget: self.min_budget });
        }
        if !(self.shrink_to > 0.0 && self.shrink_to <= 1.0) {
            return Err(LimitsError::ShrinkToOutOfRange);
        }
        if self.prefetch_from > self.prefetch_to {
            return Err(LimitsError::PrefetchWindowInverted { from: self.prefetch_from, to: self.prefetch_to });
        }
        Ok(())
    }

    pub fn encode(&self) -> String {
        format!(
            "budget={}\nmin_budget={}\nshrink_to={}\nmemory_budget={}\nslots={}\nprefetch_from={}\nprefetch_to={}\n\
             leading_chunks={}\nprefetch_budget_mib={}\ntimeout_seconds={}\nmax_concurrent={}\ndedupe_window_seconds={}\ndedupe_max_entries={}\n\
             playback_chunks_ahead={}\nstrategy={}\n",
            self.budget,
            self.min_budget,
            self.shrink_to,
            self.memory_budget,
            self.slots,
            self.prefetch_from,
            self.prefetch_to,
            self.leading_chunks.to_vec().iter().map(u32::to_string).collect::<Vec<_>>().join(","),
            self.prefetch_budget_mib,
            self.timeout_seconds,
            self.max_concurrent,
            self.dedupe_window_seconds,
            self.dedupe_max_entries,
            self.playback_chunks_ahead,
            self.strategy.encode(),
        )
    }

    pub fn decode(base: Limits, text: &str) -> Result<Limits, LimitsError> {
        let mut limits = base;
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let (key, value) = line.split_once('=').ok_or(LimitsError::MalformedLine { line: index + 1 })?;
            limits.apply(key.trim(), value.trim())?;
        }
        limits.validate()?;
        Ok(limits)
    }

    fn apply(&mut self, key: &str, value: &str) -> Result<(), LimitsError> {
        match key {
            "budget" => self.budget = number("budget", value)?,
            "min_budget" => self.min_budget = number("min_budget", value)?,
            "shrink_to" => self.shrink_to = number("shrink_to", value)?,
            "memory_budget" => self.memory_budget = number("memory_budget", value)?,
            "slots" => self.slots = number("slots", value)?,
            "prefetch_from" => self.prefetch_from = number("prefetch_from", value)?,
            "prefetch_to" => self.prefetch_to = number("prefetch_to", value)?,
            "leading_chunks" => self.leading_chunks = leading_chunks(value)?,
            "prefetch_budget_mib" => self.prefetch_budget_mib = number("prefetch_budget_mib", value)?,
            "timeout_seconds" => self.timeout_seconds = number("timeout_seconds", value)?,
            "max_concurrent" => self.max_concurrent = number("max_concurrent", value)?,
            "dedupe_window_seconds" => self.dedupe_window_seconds = number("dedupe_window_seconds", value)?,
            "dedupe_max_entries" => self.dedupe_max_entries = number("dedupe_max_entries", value)?,
            "playback_chunks_ahead" => self.playback_chunks_ahead = number("playback_chunks_ahead", value)?,
            "strategy" => self.strategy = EvictionStrategy::decode(value)?,
            _ => return Err(LimitsError::UnknownKey { key: key.to_string() }),
        }
        Ok(())
    }
}

impl EvictionStrategy {
    fn encode(&self) -> String {
        match self {
            EvictionStrategy::Lru => "lru".to_string(),
            EvictionStrategy::Fifo => "fifo".to_string(),
            EvictionStrategy::Lfu => "lfu".to_string(),
            EvictionStrategy::LargestFirst => "largest_first".to_string(),
            EvictionStrategy::ExpireAfter(seconds) => format!("expire_after:{seconds}"),
        }
    }

    fn decode(value: &str) -> Result<EvictionStrategy, LimitsError> {
        match value {
            "lru" => Ok(EvictionStrategy::Lru),
            "fifo" => Ok(EvictionStrategy::Fifo),
            "lfu" => Ok(EvictionStrategy::Lfu),
            "largest_first" => Ok(EvictionStrategy::LargestFirst),
            _ => value
                .strip_prefix("expire_after:")
                .and_then(|seconds| seconds.parse().ok())
                .map(EvictionStrategy::ExpireAfter)
                .ok_or(LimitsError::InvalidValue { key: "strategy" }),
        }
    }
}

fn leading_chunks(value: &str) -> Result<LeadingChunks, LimitsError> {
    let values: Vec<u32> = value.split(',').map(|entry| entry.trim().parse().map_err(|_| LimitsError::InvalidValue { key: "leading_chunks" })).collect::<Result<_, _>>()?;
    LeadingChunks::new(&values)
}

fn number<T: std::str::FromStr>(key: &'static str, value: &str) -> Result<T, LimitsError> {
    value.parse().map_err(|_| LimitsError::InvalidValue { key })
}
