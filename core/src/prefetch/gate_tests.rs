use super::gate::{Gate, LoadPermit};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

fn waiter(gate: &Arc<Gate>, value: usize) -> Pin<Box<dyn Future<Output = LoadPermit>>> {
    let gate = gate.clone();
    Box::pin(async move { gate.acquire(None, &distance(value), 0).await })
}

fn distance(value: usize) -> Arc<AtomicUsize> {
    Arc::new(AtomicUsize::new(value))
}

#[tokio::test]
async fn inv_prefetch_23_cancelled_acquires_leave_the_queue_while_the_permit_is_held() {
    let gate = Gate::new(1);
    let held = gate.acquire(None, &distance(0), 0).await;
    let near = distance(1);
    let mut cancelled = Vec::new();
    for chunk in 0..8 {
        let mut waiting = Box::pin({
            let gate = gate.clone();
            let near = near.clone();
            async move { gate.acquire(None, &near, chunk).await }
        });
        assert!(futures_util::poll!(waiting.as_mut()).is_pending());
        cancelled.push(waiting);
    }
    assert_eq!((gate.waiting(), gate.running()), (8, 1));
    cancelled.truncate(1);
    assert_eq!(gate.waiting(), 1);
    cancelled.clear();
    assert_eq!((gate.waiting(), gate.running()), (0, 1));
    assert_eq!(Arc::strong_count(&near), 1);
    drop(held);
    assert_eq!((gate.waiting(), gate.running()), (0, 0));
}

#[tokio::test]
async fn inv_prefetch_23_a_waiter_cancelled_after_its_permit_was_sent_returns_the_permit() {
    let gate = Gate::new(1);
    let held = gate.acquire(None, &distance(0), 0).await;
    let mut waiting = waiter(&gate, 1);
    assert!(futures_util::poll!(waiting.as_mut()).is_pending());
    drop(held);
    assert_eq!((gate.waiting(), gate.running()), (0, 1));
    drop(waiting);
    assert_eq!((gate.waiting(), gate.running()), (0, 0));
}
