/// A playing position anchored in time: the fraction `fraction` at the monotonic millisecond `at_ms`, advancing by `per_ms` fractions each millisecond (0 when the audio does not advance).
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct Playhead {
    pub fraction: f64,
    pub at_ms: u64,
    pub per_ms: f64,
}

/// One reading of the player: position, duration (0 when unknown), the monotonic millisecond of the reading, the player rate, whether it advances, and whether the position jumped on purpose (seek, new source, rewind at the end).
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct PlayheadReport {
    pub position_ms: u64,
    pub duration_ms: u64,
    pub at_ms: u64,
    pub rate: f64,
    pub playing: bool,
    pub jump: bool,
}

/// The anchor to draw after a reading. Both platforms ask this one rule.
#[uniffi::export]
pub fn playhead_next(shown: Option<Playhead>, report: PlayheadReport) -> Playhead {
    let _ = (shown, report);
    unimplemented!("playhead_next")
}

/// The fraction to draw at the monotonic millisecond `now_ms`, between 0 and 1.
#[uniffi::export]
pub fn playhead_at(playhead: Playhead, now_ms: u64) -> f64 {
    let _ = (playhead, now_ms);
    unimplemented!("playhead_at")
}

#[cfg(test)]
mod playhead_tests;
