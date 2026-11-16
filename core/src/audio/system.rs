use statig::blocking::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum System {
    Free,
    Interrupted,
    FocusLost,
    Transient,
    Ducked,
    WaitForeground,
}

#[derive(Default)]
pub(super) struct SystemMachine;

#[state_machine(initial = "State::free()")]
impl SystemMachine {
    #[state(superstate = "any")]
    fn free() -> Outcome<State> {
        Super
    }
    #[state(superstate = "any")]
    fn interrupted() -> Outcome<State> {
        Super
    }
    #[state(superstate = "any")]
    fn focus_lost() -> Outcome<State> {
        Super
    }
    #[state(superstate = "any")]
    fn transient() -> Outcome<State> {
        Super
    }
    #[state(superstate = "any")]
    fn ducked() -> Outcome<State> {
        Super
    }
    #[state(superstate = "any")]
    fn wait_foreground() -> Outcome<State> {
        Super
    }
    #[superstate]
    fn any(event: &System) -> Outcome<State> {
        Transition(match event {
            System::Free => State::free(),
            System::Interrupted => State::interrupted(),
            System::FocusLost => State::focus_lost(),
            System::Transient => State::transient(),
            System::Ducked => State::ducked(),
            System::WaitForeground => State::wait_foreground(),
        })
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
