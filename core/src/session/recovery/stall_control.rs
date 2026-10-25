use super::{Action, Ctx, Event, Input, Signal};
use crate::session::watchdog_policy::{evaluate, window_delay_ms, Decision, Progress, Sample};
use statig::blocking::*;

#[derive(Debug, Clone, Default)]
pub(super) struct StallControl {
    has_progress: bool,
    attempts: u32,
    cooldown_cycles: u32,
    is_escalated: bool,
}

impl StallControl {
    fn restart_epoch(&mut self) {
        self.attempts = 0;
        self.cooldown_cycles = 0;
        self.is_escalated = false;
    }

    fn window(&self, ctx: &mut Ctx<'_>, position_ms: u64, is_cooling: bool) -> State {
        let delay_ms = window_delay_ms(&ctx.shared.config, self.has_progress, is_cooling);
        ctx.action = Action::Arm { delay_ms };
        let deadline_ms = ctx.now.saturating_add(delay_ms);
        if is_cooling {
            State::cooling(position_ms, deadline_ms)
        } else {
            State::watching(position_ms, deadline_ms)
        }
    }

    fn on_armed(&mut self, is_cooling: bool, window_start_ms: u64, deadline_ms: u64, ctx: &mut Ctx<'_>, event: &Input<'_>) -> Outcome<State> {
        match event {
            Input::Event(Event::Play { position_ms }) => {
                ctx.shared.wants_play = true;
                Transition(self.window(ctx, *position_ms, is_cooling))
            }
            Input::Event(Event::Pause) => {
                ctx.shared.wants_play = false;
                ctx.action = Action::Disarm;
                Transition(State::disarmed(is_cooling))
            }
            Input::Event(Event::Background) => {
                ctx.action = Action::Disarm;
                Transition(State::disarmed(is_cooling))
            }
            Input::Event(Event::Foreground) => {
                self.restart_epoch();
                if is_cooling {
                    return Transition(State::watching(window_start_ms, deadline_ms));
                }
                Handled
            }
            Input::Event(Event::WatchdogFired(sample)) => self.on_deadline(is_cooling, window_start_ms, deadline_ms, sample, ctx),
            Input::Signal(Signal::Failed) => Transition(State::disarmed(is_cooling)),
            _ => Handled,
        }
    }

    fn on_deadline(&mut self, is_cooling: bool, window_start_ms: u64, deadline_ms: u64, sample: &Sample, ctx: &mut Ctx<'_>) -> Outcome<State> {
        if ctx.now < deadline_ms {
            return Handled;
        }
        let progress = Progress {
            has_loaded: ctx.shared.has_loaded,
            attempts: self.attempts,
            is_escalated: self.is_escalated,
            is_cooldown_elapsed: is_cooling,
            cooldown_cycles: self.cooldown_cycles,
        };
        let verdict = evaluate(&ctx.shared.config, window_start_ms, sample, &progress);
        self.attempts = verdict.attempts;
        match verdict.decision {
            Decision::ResetAndRearm => {
                self.cooldown_cycles = 0;
                self.has_progress = true;
                ctx.shared.retry_index = 0;
                Transition(self.window(ctx, sample.position_ms, false))
            }
            Decision::Rearm => Transition(self.window(ctx, sample.position_ms, false)),
            Decision::Recover => {
                ctx.action = Action::Recover;
                Transition(State::disarmed(false))
            }
            Decision::Escalate => {
                self.is_escalated = true;
                ctx.signal = Some(Signal::Escalated);
                Transition(State::disarmed(false))
            }
            Decision::EnterCooldown => {
                self.cooldown_cycles += 1;
                Transition(self.window(ctx, sample.position_ms, true))
            }
            Decision::Terminate => {
                ctx.action = Action::Fail("watchdog terminal".to_string());
                Transition(State::terminal())
            }
        }
    }
}

#[state_machine(initial = "State::disarmed(false)", state(derive(Debug, Clone)))]
impl StallControl {
    #[state]
    fn disarmed(&mut self, next_is_cooling: &mut bool, context: &mut Ctx<'_>, event: &Input<'_>) -> Outcome<State> {
        match event {
            Input::Event(Event::Play { position_ms }) => {
                context.shared.wants_play = true;
                Transition(self.window(context, *position_ms, *next_is_cooling))
            }
            Input::Event(Event::Pause) => {
                context.shared.wants_play = false;
                Handled
            }
            Input::Event(Event::Background) => {
                context.action = Action::Disarm;
                Handled
            }
            Input::Event(Event::Foreground) => {
                self.restart_epoch();
                *next_is_cooling = false;
                Handled
            }
            _ => Handled,
        }
    }

    #[state]
    fn watching(&mut self, window_start_ms: &mut u64, deadline_ms: &mut u64, context: &mut Ctx<'_>, event: &Input<'_>) -> Outcome<State> {
        self.on_armed(false, *window_start_ms, *deadline_ms, context, event)
    }

    #[state]
    fn cooling(&mut self, window_start_ms: &mut u64, deadline_ms: &mut u64, context: &mut Ctx<'_>, event: &Input<'_>) -> Outcome<State> {
        self.on_armed(true, *window_start_ms, *deadline_ms, context, event)
    }

    #[state]
    fn terminal(&mut self, context: &mut Ctx<'_>, event: &Input<'_>) -> Outcome<State> {
        match event {
            Input::Event(Event::Pause) => {
                context.shared.wants_play = false;
                Handled
            }
            Input::Event(Event::Background) => {
                context.action = Action::Disarm;
                Handled
            }
            Input::Event(Event::Foreground) => {
                self.restart_epoch();
                Transition(State::disarmed(false))
            }
            _ => Handled,
        }
    }
}
