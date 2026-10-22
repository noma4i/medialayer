use super::entry::Entry;
use super::evict::trigger_eviction;
use super::inner::{delete_files, CacheShared};
use super::naming::{bin_path, file_name, name_of_bin_file, INDEX_FILE, PART_SUFFIX};
use super::persist::{decode_line, HEADER};
use super::save::mark_dirty;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;

const LOAD_BATCH: usize = 512;

struct Listing {
    bins: HashSet<String>,
    parts: Vec<PathBuf>,
}

pub async fn load(inner: Arc<CacheShared>) {
    let _ = tokio::fs::create_dir_all(&inner.dir).await;
    let dir = inner.dir.clone();
    let listing = tokio::task::spawn_blocking(move || list_dir(&dir)).await.unwrap_or(Listing { bins: HashSet::new(), parts: Vec::new() });
    if let Ok(file) = tokio::fs::File::open(inner.dir.join(INDEX_FILE)).await {
        let mut reader = tokio::io::BufReader::new(file);
        let mut line = String::new();
        let mut batch = Vec::with_capacity(LOAD_BATCH);
        if reader.read_line(&mut line).await.is_ok() && line == HEADER {
            let mut valid_len = line.len() as u64;
            loop {
                line.clear();
                if !matches!(reader.read_line(&mut line).await, Ok(length) if length > 0) || !line.ends_with('\n') {
                    break;
                }
                valid_len += line.len() as u64;
                let line = line.trim_end_matches('\n');
                if let Some(name) = line.strip_prefix('-') {
                    batch.push((name.to_owned(), None));
                } else if let Some((key, entry)) = decode_line(line) {
                    batch.push((key, Some(entry)));
                }
                if batch.len() == LOAD_BATCH {
                    merge(&inner, &batch, &listing.bins);
                    batch.clear();
                    tokio::task::yield_now().await;
                }
            }
            merge(&inner, &batch, &listing.bins);
            let mut saved = inner.persist_lock.lock().await;
            saved.valid_len = valid_len;
            saved.snapshot_bytes = valid_len;
        }
    }
    inner.ready.send_replace(true);
    trigger_eviction(&inner);
    remove_leftovers(&inner, listing).await;
    mark_dirty(&inner);
}

fn merge(inner: &CacheShared, batch: &[(String, Option<Entry>)], bins: &HashSet<String>) {
    let hashed: Vec<_> = batch.iter().map(|record| (if record.1.is_some() { file_name(&record.0) } else { record.0.clone() }, record)).collect();
    let mut state = inner.state();
    for (name, (key, entry)) in hashed {
        if state.slot(&name).is_some_and(|slot| slot.verified) {
            continue;
        }
        if let Some(entry) = entry {
            if bins.contains(&name) {
                state.insert(name.clone(), key.clone(), entry.clone(), false);
            }
        } else {
            state.take(&name);
        }
        state.changes.remove(&name);
    }
}

fn list_dir(dir: &Path) -> Listing {
    let mut listing = Listing { bins: HashSet::new(), parts: Vec::new() };
    let Ok(read) = std::fs::read_dir(dir) else { return listing };
    for item in read.flatten() {
        let file = item.file_name().to_string_lossy().to_string();
        if file.ends_with(PART_SUFFIX) {
            listing.parts.push(item.path());
        } else if let Some(name) = name_of_bin_file(&file) {
            listing.bins.insert(name);
        }
    }
    listing
}

async fn remove_leftovers(inner: &Arc<CacheShared>, listing: Listing) {
    let dead_parts: Vec<PathBuf> = {
        let state = inner.state();
        listing.parts.into_iter().filter(|path| !state.live_parts.contains(path)).collect()
    };
    delete_files(dead_parts).await;
    for name in listing.bins {
        remove_orphan(inner, &name).await;
        tokio::task::yield_now().await;
    }
}

async fn remove_orphan(inner: &Arc<CacheShared>, name: &str) {
    let _stripe = inner.stripe(name).lock().await;
    if inner.state().contains(name) {
        return;
    }
    let _ = tokio::fs::remove_file(bin_path(&inner.dir, name)).await;
}
