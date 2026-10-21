use super::ring::Ring;
use super::retention::Retention;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReason {
    Timer,
    Pressure,
    Background,
    SourceChange,
    Explicit,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Released {
    pub timer: u64,
    pub pressure: u64,
    pub background: u64,
    pub source_change: u64,
    pub explicit: u64,
}

impl Released {
    pub(super) fn count(&mut self, reason: ReleaseReason) {
        match reason {
            ReleaseReason::Timer => self.timer += 1,
            ReleaseReason::Pressure => self.pressure += 1,
            ReleaseReason::Background => self.background += 1,
            ReleaseReason::SourceChange => self.source_change += 1,
            ReleaseReason::Explicit => self.explicit += 1,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PoolCounters {
    pub reuses: u64,
    pub released: Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolStats {
    pub hot: usize,
    pub playing: usize,
    pub warm: usize,
    pub cold: usize,
    pub reuses: u64,
    pub released: Released,
    pub memory_bytes: u64,
}

impl Ring {
    pub fn stats(&self) -> PoolStats {
        let states = self.states();
        let count = |retention: Retention| states.iter().filter(|state| state.retention == retention).count();
        PoolStats {
            hot: count(Retention::Hot),
            playing: self.bindings.iter().flatten().filter(|binding| binding.is_playing || binding.is_peeking).count(),
            warm: count(Retention::Warm),
            cold: count(Retention::Cold),
            reuses: self.counters.reuses,
            released: self.counters.released,
            memory_bytes: self.bindings.iter().flatten().map(|binding| binding.reservation.bytes()).sum(),
        }
    }
}
