use super::loader::{LoadError, Loader};
use crate::proxy::Priority;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetch {
    Whole,
    FirstBlock,
    AllBlocks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub url: String,
    pub fetch: Fetch,
    pub priority: Priority,
}

#[derive(Default)]
struct Script {
    calls: Vec<Call>,
    bodies: HashMap<String, Vec<u8>>,
    failing: HashSet<String>,
    hanging: HashSet<String>,
    sizes: HashMap<String, u64>,
    evicted: Vec<String>,
    kept: Vec<String>,
    alive: Vec<String>,
}

#[derive(Default)]
pub struct MockLoader {
    script: Mutex<Script>,
    hang_all: AtomicBool,
    active: AtomicUsize,
    peak: AtomicUsize,
    stored_reads: AtomicUsize,
    started: tokio::sync::Notify,
    released: tokio::sync::Notify,
}

struct Active<'a>(&'a MockLoader, String);

impl<'a> Active<'a> {
    fn enter(loader: &'a MockLoader, url: &str) -> Active<'a> {
        let now = loader.active.fetch_add(1, Ordering::SeqCst) + 1;
        loader.peak.fetch_max(now, Ordering::SeqCst);
        loader.script().alive.push(url.to_string());
        Active(loader, url.to_string())
    }
}

impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        let mut script = self.0.script();
        if let Some(position) = script.alive.iter().position(|url| *url == self.1) {
            script.alive.remove(position);
        }
    }
}

impl MockLoader {
    pub fn serve(&self, url: &str, body: &str) {
        self.script().bodies.insert(url.to_string(), body.as_bytes().to_vec());
    }

    pub fn hang(&self, url: &str) {
        self.script().hanging.insert(url.to_string());
    }

    pub async fn started(&self) {
        self.started.notified().await;
    }

    pub fn release(&self, url: &str) {
        self.script().hanging.remove(url);
        self.released.notify_one();
    }

    pub fn size(&self, url: &str, bytes: u64) {
        self.script().sizes.insert(url.to_string(), bytes);
    }

    pub fn evicted(&self) -> Vec<String> {
        self.script().evicted.clone()
    }

    pub fn kept(&self) -> Vec<String> {
        self.script().kept.clone()
    }

    pub fn hang_all(&self) {
        self.hang_all.store(true, Ordering::SeqCst);
    }

    pub fn fail(&self, url: &str, failing: bool) {
        let mut script = self.script();
        if failing {
            script.failing.insert(url.to_string());
        } else {
            script.failing.remove(url);
        }
    }

    pub fn calls(&self) -> Vec<Call> {
        self.script().calls.clone()
    }

    pub fn urls(&self) -> Vec<String> {
        self.calls().into_iter().map(|call| call.url).collect()
    }

    pub fn alive(&self) -> Vec<String> {
        self.script().alive.clone()
    }

    pub fn active(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    pub fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    pub fn stored_reads(&self) -> usize {
        self.stored_reads.load(Ordering::SeqCst)
    }

    fn script(&self) -> MutexGuard<'_, Script> {
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn run(&self, url: &str, fetch: Fetch, priority: Priority) -> Result<u64, LoadError> {
        let _active = Active::enter(self, url);
        let (failing, hanging, bytes) = {
            let mut script = self.script();
            script.calls.push(Call { url: url.to_string(), fetch, priority });
            (script.failing.contains(url), script.hanging.contains(url), script.sizes.get(url).copied().unwrap_or(1))
        };
        self.started.notify_one();
        tokio::task::yield_now().await;
        if hanging || self.hang_all.load(Ordering::SeqCst) {
            self.released.notified().await;
        }
        if failing {
            return Err(LoadError(format!("{url} refused")));
        }
        Ok(bytes)
    }
}

#[async_trait]
impl Loader for MockLoader {
    async fn load(&self, url: &str, _headers: &HashMap<String, String>, priority: Priority) -> Result<u64, LoadError> {
        self.run(url, Fetch::Whole, priority).await
    }

    async fn load_first_block(
        &self,
        url: &str,
        _headers: &HashMap<String, String>,
        priority: Priority,
    ) -> Result<u64, LoadError> {
        self.run(url, Fetch::FirstBlock, priority).await
    }

    async fn load_all_blocks(
        &self,
        url: &str,
        _headers: &HashMap<String, String>,
        priority: Priority,
    ) -> Result<u64, LoadError> {
        self.run(url, Fetch::AllBlocks, priority).await
    }

    async fn keep(&self, url: &str, _headers: &HashMap<String, String>) {
        self.script().kept.push(url.to_string());
    }

    async fn stored(&self, url: &str, _headers: &HashMap<String, String>) -> Option<Vec<u8>> {
        self.stored_reads.fetch_add(1, Ordering::SeqCst);
        self.script().bodies.get(url).cloned()
    }

    async fn stored_len(&self, url: &str, _headers: &HashMap<String, String>) -> Option<u64> {
        let script = self.script();
        script.calls.iter().any(|call| call.url == url).then(|| script.sizes.get(url).copied().unwrap_or(1))
    }

    async fn evict(&self, url: &str, _headers: &HashMap<String, String>) {
        self.script().evicted.push(url.to_string());
    }
}
