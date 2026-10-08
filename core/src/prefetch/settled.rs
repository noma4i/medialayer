#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Loaded,
    Failed(String),
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    pub url: String,
    pub outcome: Outcome,
}
