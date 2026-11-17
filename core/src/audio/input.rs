use statig::blocking::*;
use super::{AudioAction, RecordingKind};

#[derive(Clone)]
pub(super) struct Life;

pub(super) enum InputEvent {
    Wake,
    Block,
    Sound { generation: u64, heard: bool },
    Failed { generation: u64 },
}

pub(super) struct InputContext<'a> {
    pub id: u64,
    pub kind: RecordingKind,
    pub generation: &'a mut u64,
    pub actions: &'a mut Vec<AudioAction>,
    pub woke: &'a mut bool,
}

fn stop(context: &mut InputContext<'_>) -> Outcome<State> {
    context.actions.push(AudioAction::StopInput { id: context.id, generation: *context.generation });
    Transition(State::waiting())
}

#[state_machine(initial = "State::waiting()", state(derive(Clone)))]
impl Life {
    #[state]
    fn waiting(event: &InputEvent, context: &mut InputContext<'_>) -> Outcome<State> {
        match event {
            InputEvent::Wake => {
                *context.generation += 1;
                context.actions.push(AudioAction::RestoreInput { id: context.id, kind: context.kind, generation: *context.generation });
                Transition(State::connecting())
            }
            _ => Handled,
        }
    }

    #[state]
    fn connecting(event: &InputEvent, context: &mut InputContext<'_>) -> Outcome<State> {
        match event {
            InputEvent::Sound { generation, heard: true } if *generation == *context.generation => Transition(State::live()),
            InputEvent::Failed { generation } if *generation == *context.generation => Transition(State::waiting()),
            InputEvent::Block => stop(context),
            _ => Handled,
        }
    }

    #[state]
    fn live(event: &InputEvent, context: &mut InputContext<'_>) -> Outcome<State> {
        match event {
            InputEvent::Sound { generation, heard: false } if *generation == *context.generation => {
                *context.woke = true;
                Transition(State::waiting())
            }
            InputEvent::Block => stop(context),
            _ => Handled,
        }
    }
}

pub(super) fn is_live(machine: &StateMachine<Life>) -> bool {
    matches!(machine.state(), State::Live {})
}
