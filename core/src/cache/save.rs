use super::inner::CacheShared;
use super::naming::{INDEX_FILE, INDEX_TMP_FILE};
use super::persist::{encode_line, HEADER};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

const SAVE_BATCH: usize = 1000;

#[derive(Default)]
pub struct SavedIndex {
    pub valid_len: u64,
    pub snapshot_bytes: u64,
    journal_bytes: u64,
    text: String,
}

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
    let _ = inner.ready.subscribe().wait_for(|ready| *ready).await;
    let mut saved = inner.persist_lock.lock().await;
    #[cfg(test)]
    inner.persist_attempts.fetch_add(1, Ordering::Relaxed);
    inner.dirty.store(false, Ordering::SeqCst);
    let mut pending = PendingSave { dirty: &inner.dirty, complete: false };
    let revision = inner.state().revision;
    let result = if saved.valid_len == 0 || saved.journal_bytes >= saved.snapshot_bytes {
        write_snapshot(inner, &mut saved).await
    } else {
        append_changes(inner, &mut saved, revision).await
    };
    if result.is_ok() {
        inner.state().changes.retain(|_, changed| *changed > revision);
    }
    pending.complete = result.is_ok();
    result
}

async fn write_snapshot(inner: &Arc<CacheShared>, saved: &mut SavedIndex) -> std::io::Result<()> {
    let temporary = inner.dir.join(INDEX_TMP_FILE);
    let mut file = tokio::fs::File::create(&temporary).await?;
    write_bytes(inner, &mut file, HEADER.as_bytes()).await?;
    let mut length = HEADER.len() as u64;
    let text = &mut saved.text;
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
        write_bytes(inner, &mut file, text.as_bytes()).await?;
        length += text.len() as u64;
        after = chunk.last().map(|item| item.name.clone());
        if chunk.len() < SAVE_BATCH {
            break;
        }
        tokio::task::yield_now().await;
    }
    file.flush().await?;
    drop(file);
    tokio::fs::rename(&temporary, inner.dir.join(INDEX_FILE)).await?;
    saved.valid_len = length;
    saved.snapshot_bytes = length;
    saved.journal_bytes = 0;
    Ok(())
}

async fn append_changes(inner: &Arc<CacheShared>, saved: &mut SavedIndex, revision: u64) -> std::io::Result<()> {
    let mut file = tokio::fs::OpenOptions::new().write(true).open(inner.dir.join(INDEX_FILE)).await?;
    file.set_len(saved.valid_len).await?;
    file.seek(std::io::SeekFrom::Start(saved.valid_len)).await?;
    let mut after: Option<String> = None;
    let mut added = 0;
    loop {
        saved.text.clear();
        let count = {
            let state = inner.state();
            let lower = after.as_deref().map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
            let mut count = 0;
            let mut last = None;
            for (name, changed) in state.changes.range::<str, _>((lower, std::ops::Bound::Unbounded)).take(SAVE_BATCH) {
                if *changed <= revision {
                    if let Some(slot) = state.slot(name) {
                        encode_line(&slot.key, &slot.entry, &mut saved.text);
                    } else {
                        saved.text.push('-');
                        saved.text.push_str(name);
                        saved.text.push('\n');
                    }
                }
                last = Some(name.clone());
                count += 1;
            }
            after = last;
            count
        };
        write_bytes(inner, &mut file, saved.text.as_bytes()).await?;
        added += saved.text.len() as u64;
        if count < SAVE_BATCH {
            break;
        }
        tokio::task::yield_now().await;
    }
    file.flush().await?;
    saved.valid_len += added;
    saved.journal_bytes += added;
    Ok(())
}

async fn write_bytes(_inner: &CacheShared, file: &mut tokio::fs::File, bytes: &[u8]) -> std::io::Result<()> {
    file.write_all(bytes).await?;
    #[cfg(test)]
    _inner.index_bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
    Ok(())
}
