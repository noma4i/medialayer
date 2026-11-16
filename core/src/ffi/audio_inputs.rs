use super::ingress::{AudioOp, CoreAudio, Declared};
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
        let mut pauses = false;
        let mut barrier_position = None;
        for action in next.step(input) {
            match action {
                AudioAction::PauseAll { reason } => { pauses = true; commands.push(AudioOp::PauseAll(reason)); }
                AudioAction::ResumePaused => commands.push(AudioOp::ResumePaused),
                AudioAction::ForgetPaused => commands.push(AudioOp::ForgetPaused),
                AudioAction::CallHold { on } => { pauses |= on; commands.push(AudioOp::CallHold(on)); }
                _ => {
                    if pauses && barrier_position.is_none() && matches!(action, AudioAction::Apply { .. } | AudioAction::Deactivate { .. }) {
                        barrier_position = Some(platform.len());
                    }
                    platform.push(action);
                }
            }
        }
        if let Some(position) = barrier_position {
            let barrier = closing.queue.next_pause();
            closing.queue.submit_audio(vec![Declared::Audio(CoreAudio::Barrier(commands, barrier, closing.queue.pause_progress()))])?;
            platform.insert(position, AudioAction::AwaitPaused { barrier });
        } else {
            closing.queue.submit_audio(commands.into_iter().map(|op| Declared::Audio(CoreAudio::Op(op))).collect())?;
        }
        *owner = next;
        Ok(platform)
    }

    pub async fn paused(&self, barrier: u64) {
        let Some(closing) = self.closing.upgrade() else { return };
        let mut progress = closing.queue.pause_progress().subscribe();
        let mut ended = closing.queue.ended();
        tokio::select! {
            _ = progress.wait_for(|(processed, slots)| *processed >= barrier && slots.values().all(|(pending, acked)| *acked >= barrier.min(*pending))) => {},
            _ = ended.wait_for(|ended| *ended) => {},
        }
    }
}
