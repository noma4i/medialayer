#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchdogConfig {
    pub heal_after_ms: u64,
    pub slot_timeout_ms: u64,
    pub slot_no_ready_limit: u32,
    pub first_window_ms: u64,
    pub progress_window_ms: u64,
    pub cooldown_ms: u64,
    pub progress_threshold_ms: u64,
    pub near_end_ms: u64,
    pub max_attempts: u32,
    pub max_cooldown_cycles: u32,
    pub retry_delays_ms: Vec<u64>,
    pub startup_delay_ms: u64,
    pub foreground_delays_ms: Vec<u64>,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            heal_after_ms: 6_000,
            slot_timeout_ms: 8_000,
            slot_no_ready_limit: 2,
            first_window_ms: 12_000,
            progress_window_ms: 3_000,
            cooldown_ms: 15_000,
            progress_threshold_ms: 10,
            near_end_ms: 250,
            max_attempts: 5,
            max_cooldown_cycles: 2,
            retry_delays_ms: vec![250, 1_000, 3_000, 8_000, 15_000],
            startup_delay_ms: 250,
            foreground_delays_ms: vec![100, 300, 800, 2_000, 5_000, 10_000],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub is_looping: bool,
    pub view_attached: bool,
    pub frame_rendered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub has_loaded: bool,
    pub attempts: u32,
    pub is_escalated: bool,
    pub is_cooldown_elapsed: bool,
    pub cooldown_cycles: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    ResetAndRearm,
    Recover,
    Rearm,
    Escalate,
    EnterCooldown,
    Terminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub decision: Decision,
    pub attempts: u32,
}

pub fn window_delay_ms(config: &WatchdogConfig, has_progress: bool, is_cooling: bool) -> u64 {
    if is_cooling {
        config.cooldown_ms
    } else if has_progress {
        config.progress_window_ms
    } else {
        config.first_window_ms
    }
}

pub fn has_clock_advanced(config: &WatchdogConfig, start_ms: u64, current_ms: u64, is_looping: bool) -> bool {
    (is_looping && current_ms < start_ms) || current_ms > start_ms.saturating_add(config.progress_threshold_ms)
}

pub fn evaluate(config: &WatchdogConfig, start_ms: u64, sample: &Sample, progress: &Progress) -> Verdict {
    let advanced = has_clock_advanced(config, start_ms, sample.position_ms, sample.is_looping);
    let is_near_end = !sample.is_looping
        && sample.duration_ms.is_some_and(|duration| sample.position_ms >= duration.saturating_sub(config.near_end_ms));
    let is_visible = !sample.view_attached || sample.frame_rendered;
    let verdict = |decision, attempts| Verdict { decision, attempts };
    if (advanced && is_visible) || is_near_end {
        return verdict(Decision::ResetAndRearm, 0);
    }
    let recover = if progress.has_loaded { Decision::Recover } else { Decision::Rearm };
    if progress.is_cooldown_elapsed {
        if progress.cooldown_cycles >= config.max_cooldown_cycles {
            return verdict(Decision::Terminate, progress.attempts);
        }
        return verdict(recover, 0);
    }
    let next = progress.attempts + 1;
    if next >= config.max_attempts {
        return if progress.is_escalated {
            verdict(Decision::EnterCooldown, config.max_attempts)
        } else {
            verdict(Decision::Escalate, 0)
        };
    }
    verdict(recover, next)
}
