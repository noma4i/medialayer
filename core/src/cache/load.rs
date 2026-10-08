use super::entry::Entry;
use super::evict::trigger_eviction;
use super::inner::{delete_files, CacheShared};
use super::naming::{bin_path, file_name, name_of_bin_file, INDEX_FILE, PART_SUFFIX};
use super::persist::decode_line;
use super::save::mark_dirty;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const LOAD_BATCH: usize = 512;

struct Listing {
    bins: HashMap<String, u64>,
    parts: Vec<PathBuf>,
}

pub async fn load(inner: Arc<CacheShared>) {
    let _ = tokio::fs::create_dir_all(&inner.dir).await;
    let text = tokio::fs::read_to_string(inner.dir.join(INDEX_FILE)).await.unwrap_or_default();
    let dir = inner.dir.clone();
    let listing = tokio::task::spawn_blocking(move || list_dir(&dir)).await.unwrap_or(Listing { bins: HashMap::new(), parts: Vec::new() });
    let records: Vec<(String, Entry)> = text.lines().filter_map(decode_line).collect();
    for batch in records.chunks(LOAD_BATCH) {
        merge(&inner, batch, &listing.bins);
        tokio::task::yield_now().await;
    }
    inner.ready.send_replace(true);
    trigger_eviction(&inner);
    remove_leftovers(&inner, listing).await;
    mark_dirty(&inner);
}

fn merge(inner: &CacheShared, batch: &[(String, Entry)], bins: &HashMap<String, u64>) {
    let hashed: Vec<(String, &(String, Entry))> = batch.iter().map(|record| (file_name(&record.0), record)).collect();
    let mut state = inner.state();
    for (name, (key, entry)) in hashed {
        let Some(size) = bins.get(&name) else { continue };
        if !state.contains(&name) {
            state.insert(name, key.clone(), Entry { size: *size, ..entry.clone() });
        }
    }
}

fn list_dir(dir: &Path) -> Listing {
    let mut listing = Listing { bins: HashMap::new(), parts: Vec::new() };
    let Ok(read) = std::fs::read_dir(dir) else { return listing };
    for item in read.flatten() {
        let file = item.file_name().to_string_lossy().to_string();
        if file.ends_with(PART_SUFFIX) {
            listing.parts.push(item.path());
        } else if let Some(name) = name_of_bin_file(&file) {
            listing.bins.insert(name, item.metadata().map_or(0, |meta| meta.len()));
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
    for name in listing.bins.into_keys() {
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
