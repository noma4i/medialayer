use statig::blocking::*;
use super::{AppliedResult, AudioAction, CallKind, FocusAnswer, SessionWant};

#[derive(Clone)]
pub(super) struct Session;

pub(super) enum Event {
    Reconcile { want: Option<SessionWant>, may_ask: bool, fresh: bool, next: bool },
    Applied { generation: u64, result: AppliedResult },
    Focus { generation: u64, answer: FocusAnswer },
    CallActivated(SessionWant),
    Gain,
    Other,
    Reset,
}

pub(super) struct Context<'a> {
    pub generation: &'a mut u64,
    pub actions: &'a mut Vec<AudioAction>,
}

fn own(want: &SessionWant) -> bool {
    matches!(want, SessionWant::Call { kind: CallKind::Unmanaged, .. })
}

fn ask(want: SessionWant, context: &mut Context<'_>, release: bool) -> State {
    let callkit = matches!(&want, SessionWant::Call { kind: CallKind::CallKit { .. }, .. });
    if callkit && release {
        *context.generation += 1;
        context.actions.push(AudioAction::Deactivate { generation: *context.generation });
    }
    *context.generation += 1;
    let generation = *context.generation;
    context.actions.push(AudioAction::Apply { generation, want: want.clone(), activate: !callkit });
    State::asking(generation, want)
}

fn release(context: &mut Context<'_>, own_mode: bool) -> State {
    *context.generation += 1;
    let generation = *context.generation;
    context.actions.push(AudioAction::Deactivate { generation });
    State::releasing(generation, false, own_mode)
}

#[state_machine(initial = "State::idle()", state(derive(Clone)))]
impl Session {
    #[state(superstate = "any")]
    fn idle(event: &Event, context: &mut Context<'_>) -> Outcome<State> {
        match event {
            Event::Reconcile { want: Some(want), may_ask, fresh, .. } if *may_ask || *fresh => Transition(ask(want.clone(), context, false)),
            _ => Super,
        }
    }
    #[state(superstate = "any")]
    fn asking(g: &mut u64, w: &mut SessionWant, event: &Event, context: &mut Context<'_>) -> Outcome<State> {
        match event {
            Event::Reconcile { want: None, .. } => Transition(release(context, own(w))),
            Event::Reconcile { want: Some(want), may_ask, fresh, .. } if want != w && (*may_ask || *fresh) => Transition(ask(want.clone(), context, true)),
            Event::Applied { generation, result: AppliedResult::Ok } if generation == g => Transition(State::held(w.clone())),
            Event::Focus { generation, answer: FocusAnswer::Granted } if generation == g => Transition(State::held(w.clone())),
            Event::Focus { generation, answer: FocusAnswer::Delayed } if generation == g => Transition(State::delayed(w.clone())),
            Event::Applied { generation, result: AppliedResult::Failed { .. } } if generation == g => Transition(State::refused(true)),
            Event::Focus { generation, answer: FocusAnswer::Failed } if generation == g => Transition(State::refused(false)),
            _ => Super,
        }
    }
    #[state(superstate = "any")]
    fn held(w: &mut SessionWant, event: &Event, context: &mut Context<'_>) -> Outcome<State> {
        match event {
            Event::Reconcile { want: None, .. } => Transition(release(context, own(w))),
            Event::Reconcile { want: Some(want), may_ask, fresh, .. } if want != w && (*may_ask || *fresh) => Transition(ask(want.clone(), context, true)),
            _ => Super,
        }
    }
    #[state(superstate = "any")]
    fn delayed(w: &mut SessionWant, event: &Event, context: &mut Context<'_>) -> Outcome<State> {
        match event {
            Event::Reconcile { want: None, .. } => Transition(release(context, own(w))),
            Event::Reconcile { want: Some(want), fresh: true, .. } if want != w => Transition(ask(want.clone(), context, true)),
            Event::Gain => Transition(State::held(w.clone())),
            _ => Super,
        }
    }
    #[state(superstate = "any")]
    fn refused(residue: &mut bool, event: &Event, context: &mut Context<'_>) -> Outcome<State> {
        match event {
            Event::Reconcile { want: None, .. } if *residue => Transition(release(context, false)),
            Event::Reconcile { want: None, .. } => Transition(State::idle()),
            Event::Reconcile { want: Some(want), fresh: true, .. } => Transition(ask(want.clone(), context, false)),
            _ => Super,
        }
    }
    #[state(superstate = "any")]
    fn releasing(g: &mut u64, heard: &mut bool, own_mode: &mut bool, event: &Event, context: &mut Context<'_>) -> Outcome<State> {
        match event {
            Event::Reconcile { want: Some(want), may_ask, fresh, .. } if *may_ask || *fresh => Transition(ask(want.clone(), context, false)),
            Event::Applied { generation, result: AppliedResult::Ok | AppliedResult::Failed { .. } } if generation == g => Transition(State::idle()),
            Event::Applied { generation, result: AppliedResult::Busy } if generation == g && *heard => Transition(release(context, *own_mode)),
            Event::Applied { generation, result: AppliedResult::Busy } if generation == g => Transition(State::lingering()),
            Event::Other => { *heard = true; Handled }
            _ => Super,
        }
    }
    #[state(superstate = "any")]
    fn lingering(event: &Event, context: &mut Context<'_>) -> Outcome<State> {
        match event {
            Event::Reconcile { want: Some(want), next: true, .. } => Transition(ask(want.clone(), context, false)),
            Event::Reconcile { want: None, next: true, .. } => Transition(release(context, false)),
            _ => Super,
        }
    }
    #[superstate]
    fn any(event: &Event) -> Outcome<State> {
        match event { Event::Reset => Transition(State::idle()), Event::CallActivated(want) => Transition(State::held(want.clone())), _ => Handled }
    }
}

pub(super) fn inflight(machine: &StateMachine<Session>) -> Option<u64> {
    match machine.state() { State::Asking { g, .. } | State::Releasing { g, .. } => Some(*g), _ => None }
}
pub(super) fn applied(machine: &StateMachine<Session>) -> Option<SessionWant> {
    match machine.state() { State::Asking { w, .. } | State::Held { w } | State::Delayed { w } => Some(w.clone()), _ => None }
}
pub(super) fn held(machine: &StateMachine<Session>) -> Option<SessionWant> {
    match machine.state() { State::Held { w } => Some(w.clone()), _ => None }
}
pub(super) fn blocked(machine: &StateMachine<Session>) -> bool {
    matches!(machine.state(), State::Refused { .. } | State::Delayed { .. })
}
pub(super) fn asking(machine: &StateMachine<Session>) -> bool { matches!(machine.state(), State::Asking { .. }) }
pub(super) fn lingering(machine: &StateMachine<Session>) -> bool { matches!(machine.state(), State::Lingering {}) }
pub(super) fn own_mode(machine: &StateMachine<Session>) -> bool {
    match machine.state() {
        State::Asking { w, .. } | State::Held { w } | State::Delayed { w } => own(w),
        State::Releasing { own_mode, .. } => *own_mode,
        _ => false,
    }
}
