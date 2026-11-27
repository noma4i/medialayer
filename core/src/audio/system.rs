use statig::blocking::*;
use super::{FocusAnswer, FocusChange};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum System { Free, Interrupted, FocusLost, Transient, Ducked, WaitForeground }

pub(super) enum SystemEvent {
    Audible(bool), Capture, NewDemand, CallSystemActivated, AppliedFailed, InterruptionBegan,
    InterruptionEnded(bool), ResumptionRecommendation(bool), Focus(FocusChange), FocusRequest(FocusAnswer), MediaReset, Foreground(bool),
}

#[derive(Default)]
pub(super) struct Exit { pub resume: Option<bool>, pub restore_volume: bool }

#[derive(Default, Clone)]
pub(super) struct SystemMachine;

#[state_machine(initial = "State::free()", state(derive(Clone)))]
impl SystemMachine {
    #[state(superstate = "any")]
    fn free() -> Outcome<State> { Super }
    #[state(superstate = "any")]
    fn interrupted(context: &mut Exit, event: &SystemEvent) -> Outcome<State> {
        match event {
            SystemEvent::Foreground(true) | SystemEvent::Audible(true) => { context.resume = Some(false); Transition(State::free()) }
            _ => Super,
        }
    }
    #[state(superstate = "any")]
    fn focus_lost(event: &SystemEvent) -> Outcome<State> {
        match event { SystemEvent::Audible(true) | SystemEvent::Capture | SystemEvent::NewDemand => Transition(State::free()), _ => Super }
    }
    #[state(superstate = "any")]
    fn transient(context: &mut Exit, event: &SystemEvent) -> Outcome<State> {
        match event {
            SystemEvent::Capture | SystemEvent::NewDemand => Transition(State::free()),
            SystemEvent::Focus(FocusChange::Gain) => { context.resume = Some(true); Transition(State::free()) }
            _ => Super,
        }
    }
    #[state(superstate = "any", exit_action = "restore_volume")]
    fn ducked(event: &SystemEvent) -> Outcome<State> {
        match event {
            SystemEvent::Focus(FocusChange::Gain) => Transition(State::free()),
            _ => Super,
        }
    }
    #[action]
    fn restore_volume(context: &mut Exit) { context.restore_volume = true; }
    #[state(superstate = "any")]
    fn wait_foreground(context: &mut Exit, event: &SystemEvent) -> Outcome<State> {
        match event {
            SystemEvent::Capture | SystemEvent::NewDemand => Transition(State::free()),
            SystemEvent::Foreground(true) | SystemEvent::Focus(FocusChange::Gain) => { context.resume = Some(true); Transition(State::free()) }
            _ => Super,
        }
    }
    #[superstate]
    fn any(context: &mut Exit, event: &SystemEvent) -> Outcome<State> {
        match event {
            SystemEvent::InterruptionBegan => Transition(State::interrupted()),
            SystemEvent::InterruptionEnded(resume) | SystemEvent::ResumptionRecommendation(resume) => { context.resume = Some(*resume); Transition(State::free()) }
            SystemEvent::Focus(FocusChange::Loss) => Transition(State::focus_lost()),
            SystemEvent::Focus(FocusChange::LossTransient) | SystemEvent::FocusRequest(FocusAnswer::Delayed) => Transition(State::transient()),
            SystemEvent::Focus(FocusChange::LossTransientDuck) => Transition(State::ducked()),
            SystemEvent::FocusRequest(FocusAnswer::Failed) | SystemEvent::AppliedFailed => Transition(State::wait_foreground()),
            SystemEvent::MediaReset | SystemEvent::CallSystemActivated => Transition(State::free()),
            _ => Handled,
        }
    }
}

pub(super) fn current(machine: &StateMachine<SystemMachine>) -> System {
    match machine.state() {
        State::Free {} => System::Free,
        State::Interrupted {} => System::Interrupted,
        State::FocusLost {} => System::FocusLost,
        State::Transient {} => System::Transient,
        State::Ducked {} => System::Ducked,
        State::WaitForeground {} => System::WaitForeground,
    }
}
