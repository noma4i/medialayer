use super::slots::PlayerSlot;
use crate::memory::Held;
use statig::blocking::*;
use std::sync::Arc;

pub(super) type Player = Arc<dyn PlayerSlot>;

pub(super) enum SlotSignal {
    Start(Option<Held>),
    Created(Player),
    Applied,
    Release,
    Acked,
    Dispose,
}

#[derive(Default)]
struct Life;

#[state_machine(initial = "State::empty(None)")]
impl Life {
    #[state]
    fn empty(player: &mut Option<Player>, event: &SlotSignal) -> Outcome<State> {
        match (event, player.as_ref()) {
            (SlotSignal::Start(held), None) => Transition(State::creating(held.clone())),
            (SlotSignal::Start(Some(held)), Some(player)) => Transition(State::loaded(player.clone(), held.clone(), None)),
            (SlotSignal::Release, Some(player)) => Transition(State::releasing(player.clone(), None, false)),
            (SlotSignal::Dispose, Some(player)) => Transition(State::releasing(player.clone(), None, true)),
            (SlotSignal::Dispose, None) => Transition(State::disposed()),
            _ => Handled,
        }
    }

    #[state]
    fn creating(held: &mut Option<Held>, event: &SlotSignal) -> Outcome<State> {
        match event {
            SlotSignal::Created(player) => match held.take() {
                Some(held) => Transition(State::loaded(player.clone(), held, None)),
                None => Transition(State::empty(Some(player.clone()))),
            },
            _ => Handled,
        }
    }

    #[state]
    fn loaded(player: &mut Player, live: &mut Held, previous: &mut Option<Held>, event: &SlotSignal) -> Outcome<State> {
        match event {
            SlotSignal::Start(Some(next)) => {
                *previous = Some(live.clone());
                *live = next.clone();
                Handled
            }
            SlotSignal::Applied => {
                *previous = None;
                Handled
            }
            SlotSignal::Release => Transition(State::releasing(player.clone(), Some(live.clone()), false)),
            SlotSignal::Dispose => Transition(State::releasing(player.clone(), Some(live.clone()), true)),
            _ => Handled,
        }
    }

    #[allow(unused_variables)]
    #[state]
    fn releasing(player: &mut Player, live: &mut Option<Held>, then_dispose: &mut bool, event: &SlotSignal) -> Outcome<State> {
        match event {
            SlotSignal::Acked if *then_dispose => Transition(State::disposed()),
            SlotSignal::Acked => Transition(State::empty(Some(player.clone()))),
            _ => Handled,
        }
    }

    #[state]
    fn disposed(event: &SlotSignal) -> Outcome<State> {
        let _ = event;
        Handled
    }
}

#[derive(Clone)]
pub(super) enum Start {
    Ready(Player),
    Create,
    Gone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SlotPhase {
    Empty,
    Creating,
    Loaded,
    Releasing,
    Disposed,
}

pub(super) struct SlotLife {
    machine: StateMachine<Life>,
}

impl SlotLife {
    pub fn abandon(&mut self) -> Option<Player> {
        let player = self.player();
        *self = Self::new();
        player
    }
    pub fn new() -> SlotLife {
        SlotLife { machine: Life.state_machine() }
    }

    pub fn player(&self) -> Option<Player> {
        match self.machine.state() {
            State::Empty { player } => player.clone(),
            State::Loaded { player, .. } | State::Releasing { player, .. } => Some(player.clone()),
            State::Creating { .. } | State::Disposed {} => None,
        }
    }

    pub fn phase(&self) -> SlotPhase {
        match self.machine.state() {
            State::Empty { .. } => SlotPhase::Empty,
            State::Creating { .. } => SlotPhase::Creating,
            State::Loaded { .. } => SlotPhase::Loaded,
            State::Releasing { .. } => SlotPhase::Releasing,
            State::Disposed {} => SlotPhase::Disposed,
        }
    }

    pub fn start(&mut self, held: Option<Held>) -> Start {
        self.machine.handle(&SlotSignal::Start(held));
        match (self.phase(), self.player()) {
            (SlotPhase::Disposed | SlotPhase::Releasing, _) => Start::Gone,
            (_, Some(player)) => Start::Ready(player),
            (_, None) => Start::Create,
        }
    }

    pub fn created(&mut self, player: Player) -> Player {
        self.machine.handle(&SlotSignal::Created(player.clone()));
        player
    }

    pub fn applied(&mut self) {
        self.machine.handle(&SlotSignal::Applied);
    }

    pub fn release(&mut self) -> Option<Player> {
        self.machine.handle(&SlotSignal::Release);
        (self.phase() == SlotPhase::Releasing).then(|| self.player()).flatten()
    }

    pub fn dispose(&mut self) -> Option<Player> {
        self.machine.handle(&SlotSignal::Dispose);
        (self.phase() == SlotPhase::Releasing).then(|| self.player()).flatten()
    }

    pub fn acked(&mut self) {
        self.machine.handle(&SlotSignal::Acked);
    }
}
