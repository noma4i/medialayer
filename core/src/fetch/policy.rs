use crate::limits::Limits;
use std::collections::HashMap;
use std::time::Duration;

pub const DEFAULT_FORWARDED_HEADERS: [&str; 7] = [
    "Authorization",
    "Cookie",
    "User-Agent",
    "Connection",
    "Accept",
    "Accept-Language",
    "Range",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPolicy {
    pub forwarded_headers: Vec<String>,
    pub timeout: Duration,
}

impl FetchPolicy {
    pub fn from_limits(limits: &Limits) -> Self {
        FetchPolicy {
            forwarded_headers: DEFAULT_FORWARDED_HEADERS.iter().map(|name| name.to_string()).collect(),
            timeout: Duration::from_secs(limits.timeout_seconds.into()),
        }
    }

    pub fn with_forwarded_headers(mut self, names: &[&str]) -> Self {
        self.forwarded_headers = names.iter().map(|name| name.to_string()).collect();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn forward(&self, headers: &HashMap<String, String>) -> HashMap<String, String> {
        headers
            .iter()
            .filter(|(name, _)| self.forwarded_headers.iter().any(|allowed| allowed.eq_ignore_ascii_case(name)))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }
}

impl Default for FetchPolicy {
    fn default() -> Self {
        FetchPolicy::from_limits(&Limits::default())
    }
}
