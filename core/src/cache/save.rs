use super::inner::CacheShared;
use super::naming::{INDEX_FILE, INDEX_TMP_FILE};
use super::persist::encode_line;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

const SAVE_BATCH: usize = 1000;

pub fn mark_dirty(inner: &Arc<CacheShared>) {
    inner.change.mark();
    inner.dirty.store(true, Ordering::SeqCst);
    if inner.persisting.swap(true, Ordering::SeqCst) {
        return;
    }
    inner.handle.spawn(persist_loop(inner.clone()));
}

async fn persist_loop(initial: Arc<CacheShared>) {
    let owner = Arc::downgrade(&initial);
    let delay = initial.persist_delay;
    let mut pending = Some(initial);
    loop {
        tokio::time::sleep(delay).await;
        let Some(inner) = pending.take().or_else(|| owner.upgrade()) else { return };
        while inner.dirty.swap(false, Ordering::SeqCst) {
            if write_index(&inner).await.is_err() {
                inner.dirty.store(true, Ordering::SeqCst);
                break;
            }
        }
        inner.persisting.store(false, Ordering::SeqCst);
        if !inner.dirty.load(Ordering::SeqCst) || inner.persisting.swap(true, Ordering::SeqCst) {
            return;
        }
    }
}

pub async fn write_index(inner: &Arc<CacheShared>) -> std::io::Result<()> {
    let _guard = inner.persist_lock.lock().await;
    let _ = inner.ready.subscribe().wait_for(|ready| *ready).await;
    let temporary = inner.dir.join(INDEX_TMP_FILE);
    let mut file = tokio::fs::File::create(&temporary).await?;
    let mut after: Option<String> = None;
    loop {
        let chunk = inner.state().chunk(after.as_deref(), SAVE_BATCH);
        let text: String = chunk.iter().map(|item| encode_line(&item.key, &item.entry)).collect();
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
