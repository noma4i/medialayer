use statig::blocking::*;

pub(super) enum LayerSignal {
    Installed,
    Failed,
    Shutdown,
    Cleaned,
}

#[derive(Default)]
struct Life;

#[state_machine(initial = "State::opening()")]
impl Life {
    #[state]
    fn opening(event: &LayerSignal) -> Outcome<State> {
        match event {
            LayerSignal::Installed => Transition(State::ready()),
            LayerSignal::Failed => Transition(State::failed()),
            LayerSignal::Shutdown => Transition(State::closing()),
            LayerSignal::Cleaned => Handled,
        }
    }

    #[state]
    fn ready(event: &LayerSignal) -> Outcome<State> {
        match event {
            LayerSignal::Shutdown => Transition(State::closing()),
            _ => Handled,
        }
    }

    #[state]
    fn failed(event: &LayerSignal) -> Outcome<State> {
        match event {
            LayerSignal::Shutdown => Transition(State::closing()),
            _ => Handled,
        }
    }

    #[state]
    fn closing(event: &LayerSignal) -> Outcome<State> {
        match event {
            LayerSignal::Cleaned => Transition(State::closed()),
            _ => Handled,
        }
    }

    #[state]
    fn closed(event: &LayerSignal) -> Outcome<State> {
        let _ = event;
        Handled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LayerPhase {
    Opening,
    Ready,
    Failed,
    Closing,
    Closed,
}

pub(super) struct LayerLife {
    machine: StateMachine<Life>,
}

impl LayerLife {
    pub fn new() -> LayerLife {
        LayerLife { machine: Life.state_machine() }
    }

    pub fn phase(&self) -> LayerPhase {
        match self.machine.state() {
            State::Opening {} => LayerPhase::Opening,
            State::Ready {} => LayerPhase::Ready,
            State::Failed {} => LayerPhase::Failed,
            State::Closing {} => LayerPhase::Closing,
            State::Closed {} => LayerPhase::Closed,
        }
    }

    fn moves_to(&mut self, signal: LayerSignal, phase: LayerPhase) -> bool {
        let before = self.phase();
        self.machine.handle(&signal);
        before != phase && self.phase() == phase
    }

    pub fn installed(&mut self) -> bool {
        self.moves_to(LayerSignal::Installed, LayerPhase::Ready)
    }

    pub fn failed(&mut self) -> bool {
        self.moves_to(LayerSignal::Failed, LayerPhase::Failed)
    }

    pub fn shutdown(&mut self) -> bool {
        self.moves_to(LayerSignal::Shutdown, LayerPhase::Closing)
    }

    pub fn cleaned(&mut self) -> bool {
        self.moves_to(LayerSignal::Cleaned, LayerPhase::Closed)
    }
}
