use super::AudioOp;
use crate::audio::PauseReason;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct AudioGlobal {
    pub paused: bool,
    pub pause_reason: Option<PauseReason>,
    pub in_call: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct PlaceAudio {
    pub playing: bool,
    pub peeking: bool,
    pub forgotten: bool,
    pub reason: Option<PauseReason>,
}

#[derive(Default)]
pub(super) struct AudioEffects {
    pub forget: bool,
    pub flush: bool,
    pub stop_all_playing: bool,
    pub reset_players: bool,
    pub publish: bool,
}

pub(super) fn step(global: &mut AudioGlobal, places: &mut [PlaceAudio], op: &AudioOp) -> AudioEffects {
    let mut effects = AudioEffects::default();
    match op {
        AudioOp::PauseAll(reason) => {
            global.paused = true;
            global.pause_reason = Some(*reason);
            for place in places.iter_mut().filter(|place| place.playing) { place.reason = Some(*reason); }
            effects.forget = matches!(reason, PauseReason::OutputLost | PauseReason::FocusLost | PauseReason::MediaReset);
            if effects.forget { global.paused = false; }
            effects.flush = true;
            effects.reset_players = *reason == PauseReason::MediaReset;
            effects.publish = true;
        }
        AudioOp::ResumePaused => {
            global.paused = false;
            global.pause_reason = None;
            for place in places.iter_mut().filter(|place| place.playing) { place.reason = None; }
            effects.flush = true;
            effects.publish = true;
        }
        AudioOp::ForgetPaused => {
            effects.forget = true;
            global.paused = false;
            global.pause_reason = None;
            effects.flush = true;
            effects.publish = true;
        }
        AudioOp::CallHold(true) if !global.in_call => {
            global.in_call = true;
            for place in places.iter_mut().filter(|place| place.playing) { place.reason = Some(PauseReason::InCall); }
            effects.stop_all_playing = true;
            effects.forget = true;
            effects.flush = true;
            effects.publish = true;
        }
        AudioOp::CallHold(false) => global.in_call = false,
        AudioOp::CallHold(true) => {}
    }
    if effects.forget {
        for place in places.iter_mut().filter(|place| place.reason.is_some() || (place.peeking && !place.playing)) {
            place.playing = false;
            place.peeking = false;
            place.forgotten = true;
        }
    }
    effects
}
