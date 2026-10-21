use super::inner::CacheShared;
use super::naming::{INDEX_FILE, INDEX_TMP_FILE};
use super::persist::encode_line;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

const SAVE_BATCH: usize = 1000;

struct PendingSave<'a> {
    dirty: &'a std::sync::atomic::AtomicBool,
    complete: bool,
}

impl Drop for PendingSave<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.dirty.store(true, Ordering::SeqCst);
        }
    }
}

pub fn mark_dirty(inner: &Arc<CacheShared>) {
    inner.change.mark();
    inner.generation.fetch_add(1, Ordering::SeqCst);
    inner.dirty.store(true, Ordering::SeqCst);
    if inner.persisting.swap(true, Ordering::SeqCst) {
        return;
    }
    inner.handle.spawn(persist_loop(inner.clone()));
}

async fn persist_loop(inner: Arc<CacheShared>) {
    let delay = inner.persist_delay;
    loop {
        tokio::time::sleep(delay).await;
        let generation = inner.generation.load(Ordering::SeqCst);
        if inner.dirty.load(Ordering::SeqCst) {
            let _ = write_index(&inner).await;
        }
        inner.persisting.store(false, Ordering::SeqCst);
        if inner.generation.load(Ordering::SeqCst) == generation || !inner.dirty.load(Ordering::SeqCst) || inner.persisting.swap(true, Ordering::SeqCst) {
            return;
        }
    }
}

pub async fn write_index(inner: &Arc<CacheShared>) -> std::io::Result<()> {
    let mut text = inner.persist_lock.lock().await;
    #[cfg(test)]
    inner.persist_attempts.fetch_add(1, Ordering::Relaxed);
    let _ = inner.ready.subscribe().wait_for(|ready| *ready).await;
    inner.dirty.store(false, Ordering::SeqCst);
    let mut pending = PendingSave { dirty: &inner.dirty, complete: false };
    let result = write_snapshot(inner, &mut text).await;
    pending.complete = result.is_ok();
    result
}

async fn write_snapshot(inner: &Arc<CacheShared>, text: &mut String) -> std::io::Result<()> {
    let temporary = inner.dir.join(INDEX_TMP_FILE);
    let mut file = tokio::fs::File::create(&temporary).await?;
    let mut after: Option<String> = None;
    loop {
        let chunk = inner.state().chunk(after.as_deref(), SAVE_BATCH);
        text.clear();
        for item in &chunk {
            encode_line(&item.key, &item.entry, text);
        }
        #[cfg(test)]
        {
            let pause = inner.save_pause.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
            if let Some((entered, resume)) = pause {
                let _ = entered.send(());
                let _ = resume.await;
            }
        }
        file.write_all(text.as_bytes()).await?;
        after = chunk.last().map(|item| item.name.clone());
        if chunk.len() < SAVE_BATCH {
            break;
        }
        tokio::task::yield_now().await;
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&temporary, inner.dir.join(INDEX_FILE)).await
}
