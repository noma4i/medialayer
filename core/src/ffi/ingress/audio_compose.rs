use super::command::AudioOp;
use super::audio_rule::{self, AudioGlobal, PlaceAudio};
use crate::audio::PauseReason;
use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;

type Signature = Vec<u8>;
type Canonical = HashMap<Signature, Vec<u8>>;

static CANONICAL: OnceLock<Canonical> = OnceLock::new();

#[derive(Clone, Copy)]
struct State {
    global: AudioGlobal,
    place: PlaceAudio,
    flushed: Option<(bool, bool)>,
    reset: bool,
    stopped: bool,
}

fn initial() -> Vec<State> {
    (0..128).map(|i| State {
        global: AudioGlobal {
            paused: i & 8 != 0,
            pause_reason: (i & 32 != 0).then_some(PauseReason::Suspended),
            in_call: i & 4 != 0,
        },
        place: PlaceAudio {
            playing: i & 1 != 0,
            peeking: i & 2 != 0,
            forgotten: i & 64 != 0,
            reason: (i & 16 != 0).then_some(PauseReason::Suspended),
        },
        flushed: None,
        reset: false,
        stopped: false,
    }).collect()
}

fn reason(reason: Option<PauseReason>) -> u8 {
    match reason {
        None => 0,
        Some(PauseReason::Capture) => 1,
        Some(PauseReason::Interrupted) => 2,
        Some(PauseReason::FocusLost) => 3,
        Some(PauseReason::OutputLost) => 4,
        Some(PauseReason::MediaReset) => 5,
        Some(PauseReason::InCall) => 6,
        Some(PauseReason::Suspended) => 7,
    }
}

fn signature(states: &[State]) -> Signature {
    let mut result = Vec::with_capacity(states.len() * 11);
    for state in states {
        result.extend_from_slice(&[
            state.global.paused as u8, reason(state.global.pause_reason), state.global.in_call as u8,
            state.place.playing as u8, state.place.peeking as u8, state.place.forgotten as u8,
            reason(state.place.reason),
            state.flushed.map_or(0, |(playing, peeking)| 1 + playing as u8 + 2 * peeking as u8),
            state.reset as u8, state.stopped as u8,
        ]);
    }
    result
}

fn apply(states: &mut [State], op: &AudioOp) {
    for state in states {
        let effects = audio_rule::step(&mut state.global, std::slice::from_mut(&mut state.place), op);
        if effects.flush {
            let allowed = !state.global.paused && !state.global.in_call;
            state.flushed = Some((state.place.playing && allowed, state.place.peeking && allowed));
        }
        state.reset |= effects.reset_players;
        state.stopped |= effects.stop_all_playing;
    }
}

const EVENTS: [AudioOp; 11] = [
    AudioOp::PauseAll(PauseReason::Capture),
    AudioOp::PauseAll(PauseReason::Interrupted),
    AudioOp::PauseAll(PauseReason::FocusLost),
    AudioOp::PauseAll(PauseReason::OutputLost),
    AudioOp::PauseAll(PauseReason::MediaReset),
    AudioOp::PauseAll(PauseReason::InCall),
    AudioOp::PauseAll(PauseReason::Suspended),
    AudioOp::ResumePaused,
    AudioOp::ForgetPaused,
    AudioOp::CallHold(true),
    AudioOp::CallHold(false),
];

fn canonical() -> Canonical {
    let first = initial();
    let mut result = HashMap::from([(signature(&first), Vec::new())]);
    let mut pending = VecDeque::from([(first, Vec::new())]);
    while let Some((states, path)) = pending.pop_front() {
        for code in [0, 1, 2, 3, 4, 5, 6, 8, 7, 9, 10] {
            let mut next = states.clone();
            apply(&mut next, &EVENTS[code as usize]);
            let key = signature(&next);
            if let std::collections::hash_map::Entry::Vacant(entry) = result.entry(key) {
                let mut next_path = path.clone();
                next_path.push(code);
                entry.insert(next_path.clone());
                pending.push_back((next, next_path));
            }
        }
    }
    result
}

pub(in crate::ffi::ingress) fn prepare() {
    CANONICAL.get_or_init(canonical);
}

pub(in crate::ffi::ingress) fn compose(events: Vec<AudioOp>) -> Vec<AudioOp> {
    let mut states = initial();
    for event in &events { apply(&mut states, event); }
    match CANONICAL.get_or_init(canonical).get(&signature(&states)) {
        Some(path) => path.iter().map(|code| EVENTS[*code as usize]).collect(),
        None => events,
    }
}
