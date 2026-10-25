use super::{Action, Ctx, Event, Input, Signal};
use crate::hls;
use crate::session::route_staleness::RouteStaleness;
use statig::blocking::*;

#[derive(Debug, Clone, Default)]
pub(super) struct RouteChain {
    has_used_startup: bool,
}

impl RouteChain {
    fn on_http_error(&mut self, status: i32, ctx: &mut Ctx<'_>) -> Outcome<State> {
        if !hls::is_fatal_startup_http_error(status, ctx.shared.has_loaded) {
            return Handled;
        }
        if ctx.shared.via_proxy_hls && !self.has_used_startup {
            self.has_used_startup = true;
            ctx.action = Action::RefreshRoute;
            return Transition(State::refreshing(true));
        }
        ctx.action = ctx.shared.fail(&hls::describe(status, None));
        ctx.signal = Some(Signal::Failed);
        Handled
    }
}

#[state_machine(initial = "State::idle()", state(derive(Debug, Clone)))]
impl RouteChain {
    #[state(superstate = "recovering")]
    fn idle() -> Outcome<State> {
        Super
    }

    #[state(superstate = "recovering")]
    fn purging(context: &mut Ctx<'_>, event: &Input<'_>) -> Outcome<State> {
        match event {
            Input::Event(Event::CachePurged) => {
                context.action = Action::RefreshRoute;
                Transition(State::refreshing(false))
            }
            _ => Super,
        }
    }

    #[state(superstate = "recovering")]
    fn refreshing(is_startup: &mut bool, context: &mut Ctx<'_>, event: &Input<'_>) -> Outcome<State> {
        match event {
            Input::Event(Event::RouteRefreshed) => {
                context.action = if *is_startup {
                    Action::Retry(context.shared.config.startup_delay_ms)
                } else {
                    context.shared.next_retry_delay().map_or_else(|| Action::Fail("retries exhausted".to_string()), Action::Retry)
                };
                Transition(State::idle())
            }
            _ => Super,
        }
    }

    #[superstate]
    fn recovering(&mut self, context: &mut Ctx<'_>, event: &Input<'_>) -> Outcome<State> {
        match event {
            Input::Signal(Signal::Escalated) => {
                context.action = Action::PurgeCache(context.shared.key.clone());
                Transition(State::purging())
            }
            Input::Event(Event::HttpError { status }) => self.on_http_error(*status, context),
            Input::Event(Event::PlaybackFailed { reason }) => {
                context.action = context.shared.fail(reason);
                context.signal = Some(Signal::Failed);
                Handled
            }
            Input::Event(Event::RetryDue { resolved_generation, current_generation, readiness }) => {
                if RouteStaleness::is_stale(context.shared.via_proxy_hls, *resolved_generation, *current_generation, *readiness) {
                    context.action = Action::RefreshRoute;
                    return Transition(State::refreshing(false));
                }
                context.action = Action::Replay;
                Handled
            }
            Input::Event(Event::Foreground) => {
                self.has_used_startup = false;
                context.shared.retry_index = 0;
                Transition(State::idle())
            }
            _ => Handled,
        }
    }
}
