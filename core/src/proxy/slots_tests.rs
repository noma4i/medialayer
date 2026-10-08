use super::slots::{Priority, PrioritySlots, QueueFull};
use std::time::Duration;
use tokio::time::timeout;

const SHORT: Duration = Duration::from_millis(50);

#[tokio::test]
async fn grants_up_to_max_and_queues_the_rest() {
    let slots = PrioritySlots::new(2, 0, 8);
    let first = slots.acquire(Priority::Playback).await.unwrap();
    let _second = slots.acquire(Priority::Playback).await.unwrap();
    assert_eq!(slots.in_use(), 2);
    assert!(timeout(SHORT, slots.acquire(Priority::Playback)).await.is_err());
    drop(first);
    assert!(timeout(SHORT, slots.acquire(Priority::Playback)).await.is_ok());
}

#[tokio::test]
async fn playback_overtakes_queued_prefetch() {
    let slots = PrioritySlots::new(1, 0, 8);
    let held = slots.acquire(Priority::Playback).await.unwrap();
    let low = tokio::spawn({
        let slots = slots.clone();
        async move { slots.acquire(Priority::Prefetch).await.unwrap() }
    });
    tokio::time::sleep(SHORT).await;
    let high = tokio::spawn({
        let slots = slots.clone();
        async move { slots.acquire(Priority::Playback).await.unwrap() }
    });
    tokio::time::sleep(SHORT).await;
    drop(held);
    let winner = timeout(Duration::from_secs(1), high).await.expect("playback not granted first").unwrap();
    assert!(!low.is_finished());
    drop(winner);
    timeout(Duration::from_secs(1), low).await.expect("prefetch starved").unwrap();
}

#[tokio::test]
async fn prefetch_cannot_take_the_reserved_slots() {
    let slots = PrioritySlots::new(2, 1, 8);
    let _low = slots.acquire(Priority::Prefetch).await.unwrap();
    assert!(timeout(SHORT, slots.acquire(Priority::Prefetch)).await.is_err());
    assert!(timeout(SHORT, slots.acquire(Priority::Playback)).await.is_ok());
}

#[tokio::test]
async fn a_dropped_waiter_does_not_leak_its_slot() {
    let slots = PrioritySlots::new(1, 0, 8);
    let held = slots.acquire(Priority::Playback).await.unwrap();
    assert!(timeout(SHORT, slots.acquire(Priority::Playback)).await.is_err());
    drop(held);
    assert!(timeout(SHORT, slots.acquire(Priority::Playback)).await.is_ok());
    assert_eq!(slots.in_use(), 0);
}

#[tokio::test]
async fn prefetch_is_refused_at_once_when_the_wait_queue_is_full() {
    let slots = PrioritySlots::new(1, 0, 1);
    let _held = slots.acquire(Priority::Prefetch).await.unwrap();
    let waiting = tokio::spawn({
        let slots = slots.clone();
        async move { slots.acquire(Priority::Prefetch).await.is_ok() }
    });
    tokio::time::sleep(SHORT).await;
    let refused = timeout(SHORT, slots.acquire(Priority::Prefetch)).await.expect("refusal must not wait");
    assert_eq!(refused.err(), Some(QueueFull));
    waiting.abort();
}

#[tokio::test]
async fn playback_waits_even_when_the_wait_queue_is_full() {
    let slots = PrioritySlots::new(1, 0, 0);
    let held = slots.acquire(Priority::Playback).await.unwrap();
    assert!(timeout(SHORT, slots.acquire(Priority::Playback)).await.is_err());
    drop(held);
    assert!(slots.acquire(Priority::Playback).await.is_ok());
}

#[tokio::test]
async fn abandoned_waiters_do_not_fill_the_queue() {
    let slots = PrioritySlots::new(1, 0, 1);
    let _held = slots.acquire(Priority::Prefetch).await.unwrap();
    assert!(timeout(SHORT, slots.acquire(Priority::Prefetch)).await.is_err());
    assert!(timeout(SHORT, slots.acquire(Priority::Prefetch)).await.is_err());
}

#[tokio::test]
async fn raising_the_limit_hands_slots_to_the_waiters_and_lowering_it_stops_new_grants() {
    let slots = PrioritySlots::new(1, 0, 8);
    let first = slots.acquire(Priority::Playback).await.unwrap();
    let waiter = tokio::spawn({
        let slots = slots.clone();
        async move { slots.acquire(Priority::Playback).await.map(drop) }
    });
    for _ in 0..100_000 {
        if slots.waiting() == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(slots.waiting(), 1);

    slots.set_max(2);

    let granted = tokio::time::timeout(std::time::Duration::from_secs(5), waiter).await;
    assert!(granted.is_ok_and(|joined| joined.unwrap().is_ok()));
    slots.set_max(1);
    let second = slots.acquire(Priority::Playback);
    tokio::pin!(second);
    assert!(futures_util::poll!(second.as_mut()).is_pending());
    drop(first);
    assert!(second.await.is_ok());
}
