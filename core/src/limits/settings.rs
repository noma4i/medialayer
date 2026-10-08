const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

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
    pub depth_seconds: u32,
    pub timeout_seconds: u32,
    pub max_concurrent: u32,
    pub dedupe_window_seconds: u32,
    pub dedupe_max_entries: u32,
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
            memory_budget: 64 * MIB,
            slots: 3,
            prefetch_from: 2,
            prefetch_to: 5,
            depth_seconds: 2,
            timeout_seconds: 30,
            max_concurrent: 2,
            dedupe_window_seconds: 60,
            dedupe_max_entries: 500,
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
            ("depth_seconds", self.depth_seconds.into()),
            ("timeout_seconds", self.timeout_seconds.into()),
            ("max_concurrent", self.max_concurrent.into()),
            ("dedupe_window_seconds", self.dedupe_window_seconds.into()),
            ("dedupe_max_entries", self.dedupe_max_entries.into()),
        ];
        if let Some((field, _)) = required.iter().find(|(_, value)| *value == 0) {
            return Err(LimitsError::Zero { field });
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
             depth_seconds={}\ntimeout_seconds={}\nmax_concurrent={}\ndedupe_window_seconds={}\ndedupe_max_entries={}\n\
             strategy={}\n",
            self.budget,
            self.min_budget,
            self.shrink_to,
            self.memory_budget,
            self.slots,
            self.prefetch_from,
            self.prefetch_to,
            self.depth_seconds,
            self.timeout_seconds,
            self.max_concurrent,
            self.dedupe_window_seconds,
            self.dedupe_max_entries,
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
            "depth_seconds" => self.depth_seconds = number("depth_seconds", value)?,
            "timeout_seconds" => self.timeout_seconds = number("timeout_seconds", value)?,
            "max_concurrent" => self.max_concurrent = number("max_concurrent", value)?,
            "dedupe_window_seconds" => self.dedupe_window_seconds = number("dedupe_window_seconds", value)?,
            "dedupe_max_entries" => self.dedupe_max_entries = number("dedupe_max_entries", value)?,
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

fn number<T: std::str::FromStr>(key: &'static str, value: &str) -> Result<T, LimitsError> {
    value.parse().map_err(|_| LimitsError::InvalidValue { key })
}
