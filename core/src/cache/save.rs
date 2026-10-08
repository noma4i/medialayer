use super::inner::Inner;
use super::naming::{INDEX_FILE, INDEX_TMP_FILE};
use super::persist::encode_line;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

const SAVE_BATCH: usize = 1000;

pub fn mark_dirty(inner: &Arc<Inner>) {
    inner.dirty.store(true, Ordering::SeqCst);
    if inner.persisting.swap(true, Ordering::SeqCst) {
        return;
    }
    let task_inner = inner.clone();
    inner.handle.spawn(persist_loop(task_inner));
}

async fn persist_loop(inner: Arc<Inner>) {
    loop {
        tokio::time::sleep(inner.persist_delay).await;
        while inner.dirty.swap(false, Ordering::SeqCst) {
            write_index(&inner).await;
        }
        inner.persisting.store(false, Ordering::SeqCst);
        if !inner.dirty.load(Ordering::SeqCst) || inner.persisting.swap(true, Ordering::SeqCst) {
            return;
        }
    }
}

pub async fn write_index(inner: &Arc<Inner>) {
    let _guard = inner.persist_lock.lock().await;
    let _ = inner.ready.subscribe().wait_for(|ready| *ready).await;
    let temporary = inner.dir.join(INDEX_TMP_FILE);
    let Ok(mut file) = tokio::fs::File::create(&temporary).await else {
        return;
    };
    let mut after: Option<String> = None;
    loop {
        let chunk = inner.state().chunk(after.as_deref(), SAVE_BATCH);
        let text: String = chunk.iter().map(|item| encode_line(&item.key, &item.entry)).collect();
        if file.write_all(text.as_bytes()).await.is_err() {
            return;
        }
        after = chunk.last().map(|item| item.name.clone());
        if chunk.len() < SAVE_BATCH {
            break;
        }
        tokio::task::yield_now().await;
    }
    if file.flush().await.is_ok() {
        drop(file);
        let _ = tokio::fs::rename(&temporary, inner.dir.join(INDEX_FILE)).await;
    }
}
