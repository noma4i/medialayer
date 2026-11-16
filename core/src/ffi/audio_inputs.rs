use super::ingress::{CoreAudio, Declared};
use super::layer::Closing;
use super::layer_life::LayerPhase;
use crate::api::MediaError;
use crate::audio::{AudioAction, AudioInput, AudioOwner};
use std::sync::{Arc, Mutex, PoisonError, Weak};

#[derive(uniffi::Object)]
pub struct AudioInputs {
    closing: Weak<Closing>,
    audio: Arc<Mutex<AudioOwner>>,
}

impl AudioInputs {
    pub(super) fn new(closing: &Arc<Closing>, audio: Arc<Mutex<AudioOwner>>) -> Arc<Self> {
        Arc::new(Self { closing: Arc::downgrade(closing), audio })
    }
}

#[uniffi::export]
impl AudioInputs {
    pub fn send(&self, input: AudioInput) -> Result<Vec<AudioAction>, MediaError> {
        let closing = self.closing.upgrade().ok_or(MediaError::Closed)?;
        let life = closing.life();
        if matches!(life.phase(), LayerPhase::Closing | LayerPhase::Closed) {
            return Err(MediaError::Closed);
        }
        let mut owner = self.audio.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = owner.clone();
        let mut platform = Vec::new();
        let mut commands = Vec::new();
        for action in next.step(input) {
            match action {
                AudioAction::PauseAll { .. } => commands.push(Declared::Audio(CoreAudio::PauseAll)),
                AudioAction::ResumePaused => commands.push(Declared::Audio(CoreAudio::ResumePaused)),
                AudioAction::ForgetPaused => commands.push(Declared::Audio(CoreAudio::ForgetPaused)),
                AudioAction::CallHold { on } => commands.push(Declared::Audio(CoreAudio::CallHold(on))),
                _ => platform.push(action),
            }
        }
        closing.queue.submit_audio(commands)?;
        *owner = next;
        Ok(platform)
    }
}
