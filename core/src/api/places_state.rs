use super::types::MediaState;
use statig::blocking::*;

pub(super) enum Signal {
    Loading,
    Cached,
    Ready,
    Rendered,
    Ended,
    Resumed,
    Unloaded { is_declared: bool },
    Failed(String),
    Cleared,
}

impl Signal {
    pub(super) fn from_item(state: MediaState) -> Signal {
        match state {
            MediaState::Idle => Signal::Cleared,
            MediaState::Loading => Signal::Loading,
            MediaState::Ready => Signal::Ready,
            MediaState::Rendered => Signal::Rendered,
            MediaState::Ended => Signal::Ended,
            MediaState::Failed { detail } => Signal::Failed(detail),
        }
    }
}

#[derive(Default)]
pub(super) struct Lifecycle;

impl From<&State> for MediaState {
    fn from(state: &State) -> MediaState {
        match state {
            State::Idle {} => MediaState::Idle,
            State::Loading {} => MediaState::Loading,
            State::Ready {} => MediaState::Ready,
            State::Rendered {} => MediaState::Rendered,
            State::Ended {} => MediaState::Ended,
            State::Failed { detail } => MediaState::Failed { detail: detail.clone() },
        }
    }
}

#[state_machine(initial = "State::idle()", state(derive(Debug, Clone, PartialEq, Eq)))]
impl Lifecycle {
    #[state(superstate = "lifecycle")]
    fn idle(event: &Signal) -> Outcome<State> {
        match event {
            Signal::Loading => Transition(State::loading()),
            _ => Super,
        }
    }

    #[state(superstate = "lifecycle")]
    fn loading(event: &Signal) -> Outcome<State> {
        match event {
            Signal::Loading => Handled,
            _ => Super,
        }
    }

    #[allow(unused_variables)]
    #[allow(clippy::ptr_arg, reason = "the state machine macro binds the state field type String")]
    #[state(superstate = "lifecycle")]
    fn failed(detail: &mut String, event: &Signal) -> Outcome<State> {
        match event {
            Signal::Loading => Transition(State::loading()),
            _ => Super,
        }
    }

    #[state(superstate = "lifecycle")]
    fn ready(event: &Signal) -> Outcome<State> {
        match event {
            Signal::Loading | Signal::Cached | Signal::Ready => Handled,
            Signal::Ended => Transition(State::ended()),
            _ => Super,
        }
    }

    #[state(superstate = "lifecycle")]
    fn rendered(event: &Signal) -> Outcome<State> {
        match event {
            Signal::Loading | Signal::Cached | Signal::Ready | Signal::Rendered => Handled,
            Signal::Ended => Transition(State::ended()),
            _ => Super,
        }
    }

    #[state(superstate = "lifecycle")]
    fn ended(event: &Signal) -> Outcome<State> {
        match event {
            Signal::Resumed => Transition(State::rendered()),
            Signal::Loading | Signal::Cached | Signal::Ready | Signal::Rendered | Signal::Ended => Handled,
            _ => Super,
        }
    }

    #[superstate]
    fn lifecycle(event: &Signal) -> Outcome<State> {
        match event {
            Signal::Cached | Signal::Ready => Transition(State::ready()),
            Signal::Rendered => Transition(State::rendered()),
            Signal::Failed(detail) => Transition(State::failed(detail.clone())),
            Signal::Unloaded { is_declared: true } => Transition(State::loading()),
            Signal::Unloaded { is_declared: false } | Signal::Cleared => Transition(State::idle()),
            Signal::Loading | Signal::Ended | Signal::Resumed => Handled,
        }
    }
}
