use super::route_staleness::{Readiness, RouteStaleness};
use super::watchdog_policy::{evaluate, window_delay_ms, Decision, Progress, Sample, WatchdogConfig};
use crate::hls;

pub const FOREGROUND_STEPS: [&str; 6] = ["arbitration", "transport", "audio-session", "routes", "ring-projection", "intent"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Source { key: String, via_proxy_hls: bool },
    Loaded,
    Play { position_ms: u64 },
    Pause,
    WatchdogFired(Sample),
    HttpError { status: i32 },
    PlaybackFailed { reason: String },
    CachePurged,
    RouteRefreshed,
    RetryDue { resolved_generation: Option<u64>, current_generation: u64, readiness: Readiness },
    Foreground,
    ForegroundPassDone { is_ok: bool },
    ForegroundDue,
    Background,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Arm { delay_ms: u64 },
    Disarm,
    Recover,
    PurgeCache(String),
    RefreshRoute,
    Retry(u64),
    Replay,
    Fail(String),
    ForegroundPass,
    ForegroundRetry(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Chain {
    Idle,
    Purging,
    Refreshing { is_startup: bool },
}

#[derive(Debug, Clone)]
pub struct RecoveryState {
    config: WatchdogConfig,
    key: String,
    via_proxy_hls: bool,
    has_loaded: bool,
    wants_play: bool,
    window_start_ms: u64,
    deadline_ms: Option<u64>,
    has_progress: bool,
    is_cooling: bool,
    attempts: u32,
    cooldown_cycles: u32,
    is_escalated: bool,
    is_terminal: bool,
    has_used_startup: bool,
    chain: Chain,
    retry_index: usize,
    is_foreground_running: bool,
    foreground_attempt: usize,
}

impl RecoveryState {
    pub fn new(config: WatchdogConfig) -> Self {
        Self {
            config,
            key: String::new(),
            via_proxy_hls: false,
            has_loaded: false,
            wants_play: false,
            window_start_ms: 0,
            deadline_ms: None,
            has_progress: false,
            is_cooling: false,
            attempts: 0,
            cooldown_cycles: 0,
            is_escalated: false,
            is_terminal: false,
            has_used_startup: false,
            chain: Chain::Idle,
            retry_index: 0,
            is_foreground_running: false,
            foreground_attempt: 0,
        }
    }

    fn reset_epoch(&mut self) {
        self.attempts = 0;
        self.cooldown_cycles = 0;
        self.is_cooling = false;
        self.is_escalated = false;
        self.is_terminal = false;
        self.has_used_startup = false;
        self.chain = Chain::Idle;
        self.retry_index = 0;
    }

    fn arm(&mut self, position_ms: u64, now: u64) -> Action {
        let delay_ms = window_delay_ms(&self.config, self.has_progress, self.is_cooling);
        self.window_start_ms = position_ms;
        self.deadline_ms = Some(now.saturating_add(delay_ms));
        Action::Arm { delay_ms }
    }

    fn next_retry_delay(&mut self) -> Option<u64> {
        let delay = self.config.retry_delays_ms.get(self.retry_index).copied()?;
        self.retry_index += 1;
        Some(delay)
    }

    fn fail(&mut self, reason: String) -> Action {
        self.deadline_ms = None;
        if !self.wants_play {
            return Action::Fail(reason);
        }
        self.next_retry_delay().map_or(Action::Fail(reason), Action::Retry)
    }

    fn on_fired(&mut self, sample: &Sample, now: u64) -> Action {
        let is_due = self.deadline_ms.is_some_and(|deadline| now >= deadline);
        if self.is_terminal || !self.wants_play || !is_due {
            return Action::None;
        }
        self.deadline_ms = None;
        let progress = Progress {
            has_loaded: self.has_loaded,
            attempts: self.attempts,
            is_escalated: self.is_escalated,
            is_cooldown_elapsed: self.is_cooling,
            cooldown_cycles: self.cooldown_cycles,
        };
        let verdict = evaluate(&self.config, self.window_start_ms, sample, &progress);
        self.attempts = verdict.attempts;
        match verdict.decision {
            Decision::ResetAndRearm => {
                self.is_cooling = false;
                self.cooldown_cycles = 0;
                self.has_progress = true;
                self.retry_index = 0;
                self.arm(sample.position_ms, now)
            }
            Decision::Rearm => {
                self.is_cooling = false;
                self.arm(sample.position_ms, now)
            }
            Decision::Recover => {
                self.is_cooling = false;
                Action::Recover
            }
            Decision::Escalate => {
                self.is_cooling = false;
                self.is_escalated = true;
                self.chain = Chain::Purging;
                Action::PurgeCache(self.key.clone())
            }
            Decision::EnterCooldown => {
                self.is_cooling = true;
                self.cooldown_cycles += 1;
                self.arm(sample.position_ms, now)
            }
            Decision::Terminate => {
                self.is_terminal = true;
                Action::Fail("watchdog terminal".to_string())
            }
        }
    }

    fn on_http_error(&mut self, status: i32) -> Action {
        if !hls::is_fatal_startup_http_error(status, self.has_loaded) {
            return Action::None;
        }
        if self.via_proxy_hls && !self.has_used_startup {
            self.has_used_startup = true;
            self.chain = Chain::Refreshing { is_startup: true };
            return Action::RefreshRoute;
        }
        self.fail(hls::describe(status, None))
    }

    fn on_route_refreshed(&mut self) -> Action {
        let Chain::Refreshing { is_startup } = self.chain else {
            return Action::None;
        };
        self.chain = Chain::Idle;
        if is_startup {
            return Action::Retry(self.config.startup_delay_ms);
        }
        self.next_retry_delay().map_or(Action::Fail("retries exhausted".to_string()), Action::Retry)
    }

    fn on_foreground_pass_done(&mut self, is_ok: bool) -> Action {
        if !self.is_foreground_running {
            return Action::None;
        }
        let delay = self.config.foreground_delays_ms.get(self.foreground_attempt).copied();
        match delay.filter(|_| !is_ok) {
            Some(delay) => {
                self.foreground_attempt += 1;
                Action::ForegroundRetry(delay)
            }
            None => {
                self.is_foreground_running = false;
                Action::None
            }
        }
    }
}

pub fn next_action(state: &mut RecoveryState, event: Event, now: u64) -> Action {
    match event {
        Event::Source { key, via_proxy_hls } => {
            let config = state.config.clone();
            let mut fresh = RecoveryState::new(config);
            fresh.key = key;
            fresh.via_proxy_hls = via_proxy_hls;
            *state = fresh;
            Action::None
        }
        Event::Loaded => {
            state.has_loaded = true;
            Action::None
        }
        Event::Play { position_ms } => {
            if state.is_terminal {
                return Action::None;
            }
            state.wants_play = true;
            state.arm(position_ms, now)
        }
        Event::Pause => {
            state.wants_play = false;
            match state.deadline_ms.take() {
                Some(_) => Action::Disarm,
                None => Action::None,
            }
        }
        Event::WatchdogFired(sample) => state.on_fired(&sample, now),
        Event::HttpError { status } => state.on_http_error(status),
        Event::PlaybackFailed { reason } => state.fail(reason),
        Event::CachePurged => {
            if state.chain != Chain::Purging {
                return Action::None;
            }
            state.chain = Chain::Refreshing { is_startup: false };
            Action::RefreshRoute
        }
        Event::RouteRefreshed => state.on_route_refreshed(),
        Event::RetryDue { resolved_generation, current_generation, readiness } => {
            if RouteStaleness::is_stale(state.via_proxy_hls, resolved_generation, current_generation, readiness) {
                state.chain = Chain::Refreshing { is_startup: false };
                return Action::RefreshRoute;
            }
            Action::Replay
        }
        Event::Foreground => {
            state.reset_epoch();
            state.is_foreground_running = true;
            state.foreground_attempt = 0;
            Action::ForegroundPass
        }
        Event::ForegroundPassDone { is_ok } => state.on_foreground_pass_done(is_ok),
        Event::ForegroundDue => {
            if state.is_foreground_running {
                Action::ForegroundPass
            } else {
                Action::None
            }
        }
        Event::Background => {
            state.is_foreground_running = false;
            state.deadline_ms = None;
            Action::Disarm
        }
    }
}
