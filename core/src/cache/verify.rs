use super::entry::{Check, Entry};
use super::inner::CacheShared;
use super::naming::bin_path;
use super::save::mark_dirty;
use sha2::{Digest, Sha256};

const READ_STEP: usize = 256 * 1024;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;


pub fn sum_of(hasher: Sha256) -> u64 {
    let digest = hasher.finalize();
    u64::from_be_bytes([digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7]])
}

fn file_sum(path: &Path, length: u64) -> Option<u64> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; READ_STEP.min(length as usize + 1)];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    (total == length).then(|| sum_of(hasher))
}

pub async fn verified(inner: &Arc<CacheShared>, name: &str) -> Option<Entry> {
    let (_, entry, verified) = inner.state().entry_state(name)?;
    if verified {
        return Some(entry);
    }
    check_file(inner, name).await
}

async fn check_file(inner: &Arc<CacheShared>, name: &str) -> Option<Entry> {
    let _stripe = inner.stripe(name).lock().await;
    let (key, entry, verified) = inner.state().entry_state(name)?;
    if verified {
        return Some(entry);
    }
    let (path, length) = (bin_path(&inner.dir, name), entry.size);
    let found = tokio::task::spawn_blocking(move || file_sum(&path, length)).await.ok().flatten();
    let reason = match (found, entry.check) {
        (None, _) => "length",
        (Some(found), Check::Sum(recorded)) if found != recorded => "checksum",
        (Some(found), _) => {
            inner.state().mark_verified(name, found);
            if entry.check == Check::Legacy {
                mark_dirty(inner);
            }
            return Some(Entry { check: Check::Sum(found), ..entry });
        }
    };
    crate::trace!("cache", "bad block {} {reason} size={length}", crate::trace::tail(&key));
    inner.state().take(name);
    let _ = tokio::fs::remove_file(bin_path(&inner.dir, name)).await;
    mark_dirty(inner);
    None
}
