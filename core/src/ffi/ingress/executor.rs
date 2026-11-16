use super::book::Book;
use super::command::Declared;
use super::queue::{Garbage, InputQueue, Upcoming};
use super::super::assembled::Assembled;
use super::super::ready::Ready;
use crate::api::MediaError;
use std::sync::Arc;
use tokio::sync::watch;

/// Test hook: while set the executor applies nothing.
#[derive(Clone)]
pub(crate) struct Pause(watch::Sender<bool>);

impl Pause {
    pub fn new() -> Pause {
        Pause(watch::channel(false).0)
    }

    #[cfg(test)]
    pub fn set(&self, paused: bool) {
        self.0.send_replace(paused);
    }

    async fn passed(&self) {
        let mut paused = self.0.subscribe();
        let _ = paused.wait_for(|paused| !*paused).await;
    }
}

fn answer(commands: Vec<Declared>, error: &MediaError) {
    for command in commands {
        match command {
            Declared::Query { reply, .. } => {
                let _ = reply.send(super::command::Answer::Failed(error.clone()));
            }
            Declared::Subscribe { owner: _, subscribing } => {
                use super::command::Subscribing;
                match subscribing {
                    Subscribing::Playback { listener, .. } => listener.failed(error.clone()),
                    Subscribing::State { listener, .. } => listener.failed(error.clone()),
                    Subscribing::Stats { listener } => listener.failed(error.clone()),
                    Subscribing::Tone { listener, .. } => listener.failed(error.clone()),
                }
            }
            _ => {}
        }
    }
}

/// Frees removed commands here, outside the lock and off the callers.
fn free(queue: &InputQueue) {
    let garbage = queue.take_garbage();
    if garbage.is_empty() {
        return;
    }
    let accounts: Vec<(u64, bool)> = garbage.iter().map(Garbage::account).collect();
    drop(garbage);
    queue.freed(&accounts);
}

/// The only executor of typed commands, before and after assemble.
pub(crate) async fn run(queue: Arc<InputQueue>, ready: Arc<Ready<Assembled>>, pause: Pause) {
    let mut ended = queue.ended();
    let assembled = tokio::select! {
        outcome = ready.wait() => outcome,
        _ = ended.wait_for(|ended| *ended) => Err(queue.end_error().unwrap_or(MediaError::Closed)),
    };
    let assembled = match assembled {
        Ok(assembled) => assembled,
        Err(error) => {
            queue.end(error.clone());
            let error = queue.end_error().unwrap_or(error);
            answer(queue.drain(), &error);
            return;
        }
    };
    let mut book = Book::new(assembled, queue.clone(), tokio::runtime::Handle::current());
    loop {
        pause.passed().await;
        free(&queue);
        for key in book.assembled.stage.take_playback_changes() { book.publish_key(&key); }
        match queue.next() {
            Upcoming::Release(owner, kind) => {
                book.release(owner, kind);
                queue.finish_release(owner);
            }
            Upcoming::Apply(popped) => {
                if popped.owner.is_none_or(|owner| queue.is_live(owner)) {
                    book.apply(popped.command, popped.reserve, popped.slot);
                }
            }
            Upcoming::Idle => tokio::select! {
                _ = queue.wake().notified() => {},
                _ = book.assembled.stage.playback_wake().notified() => {},
            },
            Upcoming::Ended(error) => {
                book.terminate(&error);
                answer(queue.drain(), &error);
                let mut deliveries = std::mem::take(&mut book.deliveries);
                drop(book);
                while deliveries.join_next().await.is_some() {}
                return;
            }
        }
    }
}
