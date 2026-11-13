/// A playing position anchored in time: the fraction `fraction` at the monotonic millisecond `at_ms`, advancing by `per_ms` fractions each millisecond (0 when the audio does not advance).
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct PlayheadAnchor {
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
    /// A seek is still in flight: the reported position is not the answer yet.
    pub settling: bool,
}

const LAG_MS: f64 = 1000.0;

/// The anchor to draw after a reading. Both platforms ask this one rule.
#[uniffi::export]
pub fn playhead_next(shown: Option<PlayheadAnchor>, report: PlayheadReport) -> PlayheadAnchor {
    let known = report.duration_ms > 0;
    let duration = report.duration_ms as f64;
    let fraction = if known { (report.position_ms as f64 / duration).clamp(0.0, 1.0) } else { 0.0 };
    let per_ms = if report.playing && known { report.rate / duration } else { 0.0 };
    let taken = PlayheadAnchor { fraction, at_ms: report.at_ms, per_ms };
    if let Some(shown) = shown.filter(|_| report.settling) {
        return shown;
    }
    if let Some(shown) = shown.filter(|_| !known) {
        return PlayheadAnchor { fraction: playhead_at(shown, report.at_ms), at_ms: report.at_ms, per_ms: 0.0 };
    }
    let Some(shown) = shown.filter(|_| !report.jump && known) else { return taken };
    let shown_now = playhead_at(shown, report.at_ms);
    if fraction >= shown_now || (shown_now - fraction) * duration > LAG_MS {
        return taken;
    }
    PlayheadAnchor { fraction: shown_now, ..taken }
}

/// The fraction to draw at the monotonic millisecond `now_ms`, between 0 and 1.
#[uniffi::export]
pub fn playhead_at(playhead: PlayheadAnchor, now_ms: u64) -> f64 {
    let elapsed = now_ms.saturating_sub(playhead.at_ms) as f64;
    (playhead.fraction + elapsed * playhead.per_ms).clamp(0.0, 1.0)
}

#[cfg(test)]
mod playhead_tests;
