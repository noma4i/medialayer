use super::entry::{Check, Entry};
use super::inner::CacheShared;
use super::naming::bin_path;
use super::save::mark_dirty;
use sha2::{Digest, Sha256};

const READ_STEP: usize = 256 * 1024;
use std::io::Read;

use std::fs::File;
use std::sync::Arc;
pub fn sum_of(hasher: Sha256) -> u64 {
    let digest = hasher.finalize();
    u64::from_be_bytes([digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7]])
}

fn file_sum(file: &mut File, length: u64, _inner: &CacheShared) -> Option<u64> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; READ_STEP.min(length as usize + 1)];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 { break; }
        hasher.update(&buffer[..read]);
        #[cfg(test)]
        _inner.hashed_bytes.fetch_add(read as u64, std::sync::atomic::Ordering::Relaxed);
        total += read as u64;
    }
    (total == length).then(|| sum_of(hasher))
}

pub async fn verified<T>(inner: &Arc<CacheShared>, name: &str, select: impl FnOnce(&Entry) -> T) -> Option<T> {
    {
        let state = inner.state();
        let slot = state.slot(name)?;
        if slot.verified {
            return Some(select(&slot.entry));
        }
    }
    let _stripe = inner.stripe(name).lock().await;
    if !inner.state().slot(name)?.verified {
        opened(inner, name).await?;
    }
    inner.state().slot(name).map(|slot| select(&slot.entry))
}

pub async fn opened(inner: &Arc<CacheShared>, name: &str) -> Option<File> {
    let (length, check, verified, version) = {
        let state = inner.state();
        let slot = state.slot(name)?;
        (slot.entry.size, slot.entry.check, slot.verified, slot.version)
    };
    let path = bin_path(&inner.dir, name);
    let checking = inner.clone();
    let found = tokio::task::spawn_blocking(move || {
        #[cfg(test)]
        checking.read_opens.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut file = File::open(path).map_err(|_| "length")?;
        if file.metadata().map_err(|_| "length")?.len() != length {
            return Err("length");
        }
        if !verified {
            let found = file_sum(&mut file, length, &checking).ok_or("length")?;
            let Check::Sum(recorded) = check;
            if found != recorded { return Err("checksum"); }
        }
        Ok(file)
    }).await.ok()?;
    let reason = match found {
        Ok(file) => {
            inner.state().mark_verified(name, version);
            return Some(file);
        }
        Err(reason) => reason,
    };
    if !super::evict::unlink(inner, name).await {
        return None;
    }
    let removed = inner.state().take(name);
    if let Some(slot) = removed {
        crate::trace!("cache", "bad block {} {reason} size={length}", crate::trace::tail(&slot.key));
    }
    mark_dirty(inner);
    None
}
