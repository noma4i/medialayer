use super::clock::ManualClock;
use super::*;
use crate::limits::{EvictionStrategy, Limits};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const WAIT_LIMIT: Duration = Duration::from_secs(10);

pub struct Fixture {
    pub cache: Cache,
    pub clock: Arc<ManualClock>,
    pub dir: tempfile::TempDir,
}

impl Fixture {
    pub async fn put(&self, key: &str, kind: Kind, len: usize) {
        self.clock.advance_ms(10);
        self.cache.put(key, kind, &payload(len)).await.unwrap();
    }

    pub async fn touch(&self, key: &str) {
        self.clock.advance_ms(10);
        assert!(self.cache.get(key).await.is_some());
    }
}

pub fn limits(budget: u64, shrink_to: f64, strategy: EvictionStrategy) -> Limits {
    Limits { budget, min_budget: 1, shrink_to, strategy, ..Limits::default() }
}

pub fn options(clock: Arc<ManualClock>, overrides: &[(Kind, EvictionStrategy)]) -> Options {
    Options {
        overrides: overrides.iter().copied().collect(),
        clock,
        persist_delay: Duration::from_millis(20),
        ..Options::default()
    }
}

pub async fn fixture(limits: Limits, overrides: &[(Kind, EvictionStrategy)]) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(1_000));
    let cache = Cache::open_with(dir.path(), limits, options(clock.clone(), overrides));
    cache.ready().await;
    Fixture { cache, clock, dir }
}

pub async fn fixture_with_slow_index(limits: Limits) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(1_000));
    let slow = Options { persist_delay: Duration::from_secs(3600), ..options(clock.clone(), &[]) };
    let cache = Cache::open_with(dir.path(), limits, slow);
    cache.ready().await;
    Fixture { cache, clock, dir }
}

pub fn payload(len: usize) -> Vec<u8> {
    vec![b'x'; len]
}

pub fn files_with_suffix(dir: &Path, suffix: &str) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|entry| entry.as_ref().unwrap().file_name().to_string_lossy().ends_with(suffix))
        .count()
}

#[track_caller]
pub fn wait_until<F: Fn() -> bool>(condition: F) -> impl std::future::Future<Output = ()> {
    crate::api::test_support::wait_until(condition, WAIT_LIMIT)
}

pub async fn wait_for_file_count(cache: &Cache, count: u64) {
    crate::api::test_support::eventually_async(|| async { cache.stats().await.file_count == count }).await;
}

pub async fn poll_then_drop<F: std::future::Future>(future: F, polls: usize) -> bool {
    let mut future = Box::pin(future);
    let mut remaining = polls;
    tokio::time::timeout(WAIT_LIMIT, std::future::poll_fn(|context| {
        if future.as_mut().poll(context).is_ready() {
            return std::task::Poll::Ready(true);
        }
        remaining -= 1;
        if remaining == 0 { std::task::Poll::Ready(false) } else { std::task::Poll::Pending }
    })).await.expect("future did not wake")
}
