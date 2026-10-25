mod foreground;
mod route_chain;
mod stall_control;

use self::foreground::ForegroundReturn;
use self::route_chain::RouteChain;
use self::stall_control::StallControl;
use super::watchdog_policy::{Sample, WatchdogConfig};
use crate::proxy::Readiness;
use statig::blocking::{IntoStateMachineExt, StateMachine};

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

#[derive(Debug, Clone, Copy)]
enum Signal {
    Escalated,
    Failed,
}

enum Input<'a> {
    Event(&'a Event),
    Signal(Signal),
}

#[derive(Debug, Clone)]
struct Shared {
    config: WatchdogConfig,
    key: String,
    via_proxy_hls: bool,
    has_loaded: bool,
    wants_play: bool,
    retry_index: usize,
}

impl Shared {
    fn next_retry_delay(&mut self) -> Option<u64> {
        let delay = self.config.retry_delays_ms.get(self.retry_index).copied()?;
        self.retry_index += 1;
        Some(delay)
    }

    fn fail(&mut self, reason: &str) -> Action {
        if !self.wants_play {
            return Action::Fail(reason.to_string());
        }
        self.next_retry_delay().map_or_else(|| Action::Fail(reason.to_string()), Action::Retry)
    }
}

struct Ctx<'a> {
    shared: &'a mut Shared,
    now: u64,
    action: Action,
    signal: Option<Signal>,
}

#[derive(Clone)]
pub struct RecoveryState {
    shared: Shared,
    stall: StateMachine<StallControl>,
    chain: StateMachine<RouteChain>,
    foreground: StateMachine<ForegroundReturn>,
}

impl RecoveryState {
    pub fn new(config: WatchdogConfig) -> Self {
        let shared = Shared { config, key: String::new(), via_proxy_hls: false, has_loaded: false, wants_play: false, retry_index: 0 };
        Self::with_shared(shared)
    }

    fn with_shared(shared: Shared) -> Self {
        Self {
            shared,
            stall: StallControl::default().state_machine(),
            chain: RouteChain::default().state_machine(),
            foreground: ForegroundReturn.state_machine(),
        }
    }

    fn restart(&mut self, key: String, via_proxy_hls: bool) {
        let config = std::mem::take(&mut self.shared.config);
        *self = Self::with_shared(Shared { config, key, via_proxy_hls, has_loaded: false, wants_play: false, retry_index: 0 });
    }

    fn dispatch(&mut self, event: &Event, now: u64) -> Action {
        if matches!(event, Event::Loaded) {
            self.shared.has_loaded = true;
        }
        let mut ctx = Ctx { shared: &mut self.shared, now, action: Action::None, signal: None };
        let input = Input::Event(event);
        self.stall.handle_with_context(&input, &mut ctx);
        self.chain.handle_with_context(&input, &mut ctx);
        self.foreground.handle_with_context(&input, &mut ctx);
        while let Some(signal) = ctx.signal.take() {
            let input = Input::Signal(signal);
            self.stall.handle_with_context(&input, &mut ctx);
            self.chain.handle_with_context(&input, &mut ctx);
        }
        ctx.action
    }
}

pub fn next_action(state: &mut RecoveryState, event: Event, now: u64) -> Action {
    match event {
        Event::Source { key, via_proxy_hls } => {
            state.restart(key, via_proxy_hls);
            Action::None
        }
        event => state.dispatch(&event, now),
    }
}
