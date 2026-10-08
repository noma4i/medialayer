use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync + 'static {
    fn now_ms(&self) -> u64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as u64).unwrap_or(0)
    }
}

#[cfg(test)]
pub struct ManualClock(std::sync::atomic::AtomicU64);

#[cfg(test)]
impl ManualClock {
    pub fn new(start_ms: u64) -> ManualClock {
        ManualClock(std::sync::atomic::AtomicU64::new(start_ms))
    }

    pub fn advance_ms(&self, delta: u64) {
        self.0.fetch_add(delta, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }
}
