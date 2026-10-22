use super::entry::{Check, Entry};
use super::inner::CacheShared;
use super::naming::bin_path;
use super::reader::Pin;
use super::save::mark_dirty;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::os::unix::fs::FileExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

pub(super) const SEGMENT: usize = 256 * 1024;

pub fn sum_of(hasher: Sha256) -> u64 {
    let digest = hasher.finalize();
    u64::from_be_bytes([digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7]])
}

pub struct Verified {
    sums: Arc<[u64]>,
    sized: AtomicBool,
    complete: AtomicBool,
    done: Mutex<Vec<bool>>,
    turn: tokio::sync::Mutex<()>,
}

impl Verified {
    pub fn written(check: &Check) -> Arc<Verified> {
        Arc::new(Verified::new(check, true))
    }

    pub fn loaded(check: &Check) -> Arc<Verified> {
        Arc::new(Verified::new(check, false))
    }

    fn new(check: &Check, written_here: bool) -> Verified {
        Verified {
            sums: check.0.clone(),
            sized: AtomicBool::new(written_here),
            complete: AtomicBool::new(written_here || check.0.is_empty()),
            done: Mutex::new(Vec::new()),
            turn: tokio::sync::Mutex::new(()),
        }
    }

    pub fn is_sized(&self) -> bool {
        self.sized.load(Ordering::Acquire)
    }

    fn is_complete(&self) -> bool {
        self.complete.load(Ordering::Acquire)
    }

    fn missing(&self, first: usize, last: usize) -> Vec<usize> {
        let done = self.done.lock().unwrap_or_else(PoisonError::into_inner);
        (first..=last).filter(|index| !done.get(*index).copied().unwrap_or(false)).collect()
    }

    fn mark(&self, index: usize) {
        let mut done = self.done.lock().unwrap_or_else(PoisonError::into_inner);
        if done.is_empty() {
            done.resize(self.sums.len(), false);
        }
        done[index] = true;
        if done.iter().all(|checked| *checked) {
            *done = Vec::new();
            self.complete.store(true, Ordering::Release);
        }
    }
}

fn segment_sum(file: &File, index: usize, size: u64, _inner: &CacheShared) -> Result<u64, &'static str> {
    let begin = (index * SEGMENT) as u64;
    let length = size.saturating_sub(begin).min(SEGMENT as u64) as usize;
    let mut bytes = vec![0u8; length];
    file.read_exact_at(&mut bytes, begin).map_err(|_| "length")?;
    #[cfg(test)]
    _inner.hashed_bytes.fetch_add(length as u64, Ordering::Relaxed);
    Ok(sum_of(Sha256::new().chain_update(&bytes)))
}

pub(super) async fn checked(pin: &Pin, file: &Arc<File>, size: u64, start: u64, wanted: u64) -> bool {
    let verified = &pin.verified;
    if wanted == 0 || verified.is_complete() {
        return true;
    }
    let first = (start / SEGMENT as u64) as usize;
    let last = ((start + wanted - 1) / SEGMENT as u64) as usize;
    if verified.missing(first, last).is_empty() {
        return true;
    }
    let _turn = verified.turn.lock().await;
    let missing = verified.missing(first, last);
    if missing.is_empty() {
        return true;
    }
    let (file, verifying, inner) = (file.clone(), verified.clone(), pin.inner.clone());
    let outcome = tokio::task::spawn_blocking(move || {
        for index in missing {
            if segment_sum(&file, index, size, &inner)? != verifying.sums[index] {
                return Err("checksum");
            }
            verifying.mark(index);
        }
        Ok(())
    })
    .await;
    match outcome {
        Ok(Ok(())) => true,
        Ok(Err(reason)) => {
            let _stripe = pin.inner.stripe(&pin.name).lock().await;
            reject(&pin.inner, &pin.name, pin.version, reason).await;
            false
        }
        Err(_) => false,
    }
}

pub async fn verified<T>(inner: &Arc<CacheShared>, name: &str, select: impl FnOnce(&Entry) -> T) -> Option<T> {
    {
        let state = inner.state();
        let slot = state.slot(name)?;
        if slot.verified.is_sized() {
            return Some(select(&slot.entry));
        }
    }
    let _stripe = inner.stripe(name).lock().await;
    if !inner.state().slot(name)?.verified.is_sized() {
        opened(inner, name).await?;
    }
    inner.state().slot(name).map(|slot| select(&slot.entry))
}

pub async fn opened(inner: &Arc<CacheShared>, name: &str) -> Option<File> {
    let (length, version, verified) = {
        let state = inner.state();
        let slot = state.slot(name)?;
        (slot.entry.size, slot.version, slot.verified.clone())
    };
    let path = bin_path(&inner.dir, name);
    let counting = inner.clone();
    let found = tokio::task::spawn_blocking(move || {
        #[cfg(test)]
        counting.read_opens.fetch_add(1, Ordering::Relaxed);
        let _ = &counting;
        let file = File::open(path).map_err(|_| "length")?;
        if file.metadata().map_err(|_| "length")?.len() != length {
            return Err("length");
        }
        Ok(file)
    })
    .await
    .ok()?;
    match found {
        Ok(file) => {
            verified.sized.store(true, Ordering::Release);
            Some(file)
        }
        Err(reason) => {
            reject(inner, name, version, reason).await;
            None
        }
    }
}

async fn reject(inner: &Arc<CacheShared>, name: &str, version: u64, reason: &str) {
    if !inner.state().slot(name).is_some_and(|slot| slot.version == version) {
        return;
    }
    if !super::evict::unlink(inner, name).await {
        return;
    }
    let removed = inner.state().take(name);
    if let Some(slot) = removed {
        crate::trace!("cache", "bad block {} {reason} size={}", crate::trace::tail(&slot.key), slot.entry.size);
    }
    mark_dirty(inner);
}
