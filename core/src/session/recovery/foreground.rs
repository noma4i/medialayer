use super::{Action, Ctx, Event, Input};
use statig::blocking::*;

#[derive(Debug, Clone, Default)]
pub(super) struct ForegroundReturn;

impl ForegroundReturn {
    fn on_pass_done(attempt: usize, is_ok: bool, ctx: &mut Ctx<'_>) -> Outcome<State> {
        let delay_ms = ctx.shared.config.foreground_delays_ms.get(attempt).copied();
        match delay_ms.filter(|_| !is_ok) {
            Some(delay_ms) => {
                ctx.action = Action::ForegroundRetry(delay_ms);
                Transition(State::waiting(attempt + 1))
            }
            None => Transition(State::idle()),
        }
    }

    fn on_active(attempt: usize, ctx: &mut Ctx<'_>, event: &Input<'_>) -> Outcome<State> {
        match event {
            Input::Event(Event::ForegroundPassDone { is_ok }) => Self::on_pass_done(attempt, *is_ok, ctx),
            Input::Event(Event::ForegroundDue) => {
                ctx.action = Action::ForegroundPass;
                Transition(State::checking(attempt))
            }
            Input::Event(Event::Background) => Transition(State::idle()),
            _ => Super,
        }
    }
}

#[state_machine(initial = "State::idle()", state(derive(Debug, Clone)))]
impl ForegroundReturn {
    #[state(superstate = "returning")]
    fn idle() -> Outcome<State> {
        Super
    }

    #[state(superstate = "returning")]
    fn checking(attempt: &mut usize, context: &mut Ctx<'_>, event: &Input<'_>) -> Outcome<State> {
        Self::on_active(*attempt, context, event)
    }

    #[state(superstate = "returning")]
    fn waiting(attempt: &mut usize, context: &mut Ctx<'_>, event: &Input<'_>) -> Outcome<State> {
        Self::on_active(*attempt, context, event)
    }

    #[superstate]
    fn returning(context: &mut Ctx<'_>, event: &Input<'_>) -> Outcome<State> {
        match event {
            Input::Event(Event::Foreground) => {
                context.action = Action::ForegroundPass;
                Transition(State::checking(0))
            }
            _ => Handled,
        }
    }
}
