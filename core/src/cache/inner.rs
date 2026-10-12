use super::clock::Clock;
use super::entry::Kind;
use super::state::{Item, State};
use crate::change::Change;
use crate::limits::{EvictionStrategy, Limits};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::sync::watch;

const STRIPES: usize = 16;

pub struct CacheShared {
    pub dir: PathBuf,
    pub clock: Arc<dyn Clock>,
    pub overrides: HashMap<Kind, EvictionStrategy>,
    pub chunk_size: usize,
    pub kind_priority: [Kind; 3],
    pub persist_delay: Duration,
    pub change: Arc<Change>,
    pub handle: Handle,
    pub part_sequence: AtomicU64,
    pub max_hold: AtomicU64,
    pub eviction_backlog: AtomicU64,
    pub ready: watch::Sender<bool>,
    pub dirty: AtomicBool,
    pub persisting: AtomicBool,
    pub evicting: AtomicBool,
    pub persist_lock: tokio::sync::Mutex<()>,
    state: Mutex<State>,
    stripes: Vec<Arc<tokio::sync::Mutex<()>>>,
}

impl CacheShared {
    pub fn new(dir: PathBuf, limits: Limits, options: super::options::Options, handle: Handle) -> CacheShared {
        CacheShared {
            dir,
            clock: options.clock,
            overrides: options.overrides,
            chunk_size: options.chunk_size.max(1),
            kind_priority: options.kind_priority,
            persist_delay: options.persist_delay,
            change: options.change,
            handle,
            part_sequence: AtomicU64::new(0),
            max_hold: AtomicU64::new(0),
            eviction_backlog: AtomicU64::new(0),
            ready: watch::channel(false).0,
            dirty: AtomicBool::new(false),
            persisting: AtomicBool::new(false),
            evicting: AtomicBool::new(false),
            persist_lock: tokio::sync::Mutex::new(()),
            state: Mutex::new(State::new(limits)),
            stripes: (0..STRIPES).map(|_| Arc::new(tokio::sync::Mutex::new(()))).collect(),
        }
    }

    pub fn state(&self) -> StateGuard<'_> {
        let guard = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        StateGuard { guard, acquired: Instant::now(), max_hold: &self.max_hold }
    }

    pub fn stripe(&self, key: &str) -> &Arc<tokio::sync::Mutex<()>> {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        &self.stripes[hasher.finish() as usize % STRIPES]
    }

    pub fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    pub fn is_ready(&self) -> bool {
        *self.ready.borrow()
    }
}

pub struct StateGuard<'a> {
    guard: MutexGuard<'a, State>,
    acquired: Instant,
    max_hold: &'a AtomicU64,
}

impl Deref for StateGuard<'_> {
    type Target = State;

    fn deref(&self) -> &State {
        &self.guard
    }
}

impl DerefMut for StateGuard<'_> {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.guard
    }
}

impl Drop for StateGuard<'_> {
    fn drop(&mut self) {
        self.max_hold.fetch_max(self.acquired.elapsed().as_nanos() as u64, Ordering::Relaxed);
    }
}

const SCAN_CHUNK: usize = 1000;

pub async fn scan(inner: &CacheShared) -> Vec<Item> {
    let mut items = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let chunk = inner.state().chunk(after.as_deref(), SCAN_CHUNK);
        let full = chunk.len() == SCAN_CHUNK;
        after = chunk.last().map(|item| item.name.clone());
        items.extend(chunk);
        if !full {
            return items;
        }
        tokio::task::yield_now().await;
    }
}

pub async fn delete_files(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    let _ = tokio::task::spawn_blocking(move || {
        for path in paths {
            let _ = std::fs::remove_file(path);
        }
    })
    .await;
}
