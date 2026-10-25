use super::flight::collect_limited;
use super::memory_store::MemoryStore;
use super::slots::{Priority, PrioritySlots};
use super::tasks::Tasks;
use super::{Config, ProxyShared};
use crate::fetch::mock::MockFetcher;
use crate::fetch::BodyStream;
use bytes::Bytes;
use std::sync::Arc;
use std::time::Duration;

const KIB: usize = 1024;
const RUNTIME_NOISE: usize = KIB;

fn body_of(sizes: &[usize]) -> BodyStream {
    let chunks: Vec<_> = sizes.iter().map(|size| Ok(Bytes::from(vec![7u8; *size]))).collect();
    Box::pin(futures_util::stream::iter(chunks))
}

fn shared() -> ProxyShared {
    ProxyShared::new(Config::default(), Arc::new(MockFetcher::new()), Arc::new(MemoryStore::new()), Tasks::default())
}

#[tokio::test]
async fn inv_proxy_40_collect_limited_never_allocates_more_than_its_limit() {
    let inner = shared();
    let buffers = inner.buffers();
    let cases: [(usize, &[usize], Option<usize>); 4] = [
        (4 * KIB, &[64 * KIB], None),
        (64 * KIB, &[16 * KIB, 16 * KIB, 16 * KIB, 20 * KIB], None),
        (32 * KIB, &[8 * KIB, 8 * KIB, 8 * KIB], Some(24 * KIB)),
        (64 * KIB, &[], Some(0)),
    ];
    for (limit, sizes, expected) in cases {
        let body = body_of(sizes);
        crate::alloc_probe::mark_peak();
        let collected = collect_limited(body, limit, Duration::from_secs(1), &buffers).await;
        let peak = crate::alloc_probe::peak_since_mark();
        assert_eq!(collected.as_ref().map(Vec::len), expected, "limit {limit} chunks {sizes:?}");
        assert!(peak <= limit + RUNTIME_NOISE, "limit {limit} chunks {sizes:?}: peak {peak} bytes");
        if let Some(collected) = collected {
            assert!(collected.capacity() <= limit, "capacity {} over limit {limit}", collected.capacity());
        }
    }
}

#[tokio::test]
async fn inv_proxy_41_cancelled_waiters_leave_both_queues_at_once() {
    for priority in [Priority::Playback, Priority::Prefetch] {
        let slots = PrioritySlots::new(1, 0, 8);
        let held = slots.acquire(Priority::Playback).await.unwrap();
        let mut waiting: Vec<_> = (0..4).map(|_| Box::pin({
            let slots = slots.clone();
            async move { slots.acquire(priority).await }
        })).collect();
        for future in &mut waiting {
            assert!(futures_util::poll!(future.as_mut()).is_pending());
        }
        assert_eq!(slots.waiting(), 4);
        let mut survivor = waiting.remove(2);
        waiting.clear();
        assert_eq!(slots.waiting(), 1, "{priority:?}: cancelled waiters stayed queued");
        assert_eq!(slots.in_use(), 1);
        drop(held);
        let permit = survivor.as_mut().await.unwrap();
        assert_eq!((slots.waiting(), slots.in_use()), (0, 1));
        drop(permit);
        assert_eq!(slots.in_use(), 0);
    }
}
