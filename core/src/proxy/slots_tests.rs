use super::slots::{Priority, PrioritySlots, QueueFull};
use crate::api::MediaError;
use crate::fetch::Fetcher as _;
use crate::ffi::{BodyReader, FetchRequest, FetchResponse, ForeignFetcher};
use crate::memory::InputLimits;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use tokio::time::timeout;

const SHORT: Duration = Duration::from_millis(50);

async fn wait_for_queue(slots: &PrioritySlots, count: usize) {
    timeout(Duration::from_secs(5), async {
        while slots.waiting() != count { tokio::task::yield_now().await; }
    }).await.expect("waiters did not register");
}

#[tokio::test]
async fn inv_proxy_09_limit_changes_preserve_the_configured_playback_reservation() {
    for (prefetch, playback) in [(Priority::Prefetch, Priority::Playback), (Priority::ImagePrefetch, Priority::Image)] {
        for initial in [1, 2] {
            let slots = PrioritySlots::new(initial, 1, 16);
            slots.set_max(1);
            let held = slots.acquire(prefetch).await.unwrap();
            slots.set_max(2);
            let waiting = slots.acquire(prefetch);
            tokio::pin!(waiting);
            assert!(futures_util::poll!(waiting.as_mut()).is_pending(), "prefetch consumed playback reservation");
            let player = slots.acquire(playback);
            tokio::pin!(player);
            let player = match futures_util::poll!(player.as_mut()) {
                std::task::Poll::Ready(Ok(permit)) => permit,
                _ => panic!("playback could not take its reservation"),
            };
            drop(player);
            assert!(futures_util::poll!(waiting.as_mut()).is_pending());
            drop(held);
            assert!(waiting.await.is_ok());
            assert_eq!(slots.in_use(), 0);
        }
    }
}

#[tokio::test]
async fn inv_proxy_10_grants_up_to_max_and_queues_the_rest() {
    let slots = PrioritySlots::new(2, 0, 8);
    let first = slots.acquire(Priority::Playback).await.unwrap();
    let _second = slots.acquire(Priority::Playback).await.unwrap();
    assert_eq!(slots.in_use(), 2);
    assert!(timeout(SHORT, slots.acquire(Priority::Playback)).await.is_err());
    drop(first);
    assert!(timeout(SHORT, slots.acquire(Priority::Playback)).await.is_ok());
}

#[tokio::test]
async fn inv_proxy_09_playback_overtakes_queued_prefetch() {
    let slots = PrioritySlots::new(1, 0, 8);
    let held = slots.acquire(Priority::Playback).await.unwrap();
    let low = tokio::spawn({
        let slots = slots.clone();
        async move { slots.acquire(Priority::Prefetch).await.unwrap() }
    });
    wait_for_queue(&slots, 1).await;
    let high = tokio::spawn({
        let slots = slots.clone();
        async move { slots.acquire(Priority::Playback).await.unwrap() }
    });
    wait_for_queue(&slots, 2).await;
    drop(held);
    let winner = timeout(Duration::from_secs(1), high).await.expect("playback not granted first").unwrap();
    assert!(!low.is_finished());
    drop(winner);
    timeout(Duration::from_secs(1), low).await.expect("prefetch starved").unwrap();
}

#[tokio::test]
async fn inv_proxy_09_prefetch_cannot_take_the_reserved_slots() {
    let slots = PrioritySlots::new(2, 1, 8);
    let _low = slots.acquire(Priority::Prefetch).await.unwrap();
    assert!(timeout(SHORT, slots.acquire(Priority::Prefetch)).await.is_err());
    assert!(timeout(SHORT, slots.acquire(Priority::Playback)).await.is_ok());
}

#[tokio::test]
async fn inv_proxy_10_a_dropped_waiter_does_not_leak_its_slot() {
    let slots = PrioritySlots::new(1, 0, 8);
    let held = slots.acquire(Priority::Playback).await.unwrap();
    assert!(timeout(SHORT, slots.acquire(Priority::Playback)).await.is_err());
    drop(held);
    assert!(timeout(SHORT, slots.acquire(Priority::Playback)).await.is_ok());
    assert_eq!(slots.in_use(), 0);
}

#[tokio::test]
async fn inv_proxy_09_prefetch_is_refused_at_once_when_the_wait_queue_is_full() {
    let slots = PrioritySlots::new(1, 0, 1);
    let _held = slots.acquire(Priority::Prefetch).await.unwrap();
    let waiting = tokio::spawn({
        let slots = slots.clone();
        async move { slots.acquire(Priority::Prefetch).await.is_ok() }
    });
    wait_for_queue(&slots, 1).await;
    let refused = timeout(SHORT, slots.acquire(Priority::Prefetch)).await.expect("refusal must not wait");
    assert_eq!(refused.err(), Some(QueueFull));
    waiting.abort();
}

#[tokio::test]
async fn inv_proxy_09_playback_waits_even_when_the_wait_queue_is_full() {
    let slots = PrioritySlots::new(1, 0, 0);
    let held = slots.acquire(Priority::Playback).await.unwrap();
    assert!(timeout(SHORT, slots.acquire(Priority::Playback)).await.is_err());
    drop(held);
    assert!(slots.acquire(Priority::Playback).await.is_ok());
}

#[tokio::test]
async fn inv_proxy_09_abandoned_waiters_do_not_fill_the_queue() {
    let slots = PrioritySlots::new(1, 0, 1);
    let _held = slots.acquire(Priority::Prefetch).await.unwrap();
    assert!(timeout(SHORT, slots.acquire(Priority::Prefetch)).await.is_err());
    assert!(timeout(SHORT, slots.acquire(Priority::Prefetch)).await.is_err());
}

#[tokio::test]
async fn inv_proxy_10_raising_the_limit_hands_slots_to_the_waiters_and_lowering_it_stops_new_grants() {
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

#[tokio::test]
async fn inv_proxy_10_random_tasks_never_hold_more_than_max_or_more_prefetch_than_the_split_and_leave_nothing_behind() {
    use crate::seeded::Dice;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let dice = Dice::new("inv_proxy_10_random_tasks_never_hold_more_than_max_or_more_prefetch_than_the_split_and_leave_nothing_behind");
    for round in 0..40u64 {
        let mut rolls = dice.fork(round);
        let max = rolls.between(1, 5) as usize;
        let reserved = rolls.below(3) as usize;
        let slots = PrioritySlots::new(max, reserved, rolls.between(1, 6) as usize);
        let prefetch_limit = max - reserved.min(max - 1);
        let (held, held_prefetch) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (peak, peak_prefetch, refused) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let mut tasks = Vec::new();
        for index in 0..30u64 {
            let (slots, held, held_prefetch) = (slots.clone(), held.clone(), held_prefetch.clone());
            let (peak, peak_prefetch, refused) = (peak.clone(), peak_prefetch.clone(), refused.clone());
            let mut lane = rolls.fork(index);
            tasks.push(tokio::spawn(async move {
                let priority = if lane.percent(40) { Priority::Playback } else { Priority::Prefetch };
                let patience = Duration::from_micros(lane.between(20, 3_000));
                let acquired = if lane.percent(30) { timeout(patience, slots.acquire(priority)).await.ok() } else { Some(slots.acquire(priority).await) };
                let Some(permit) = acquired else { return };
                let Ok(permit) = permit else {
                    refused.fetch_add(1, Ordering::SeqCst);
                    return;
                };
                let now = held.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                if priority == Priority::Prefetch {
                    let now = held_prefetch.fetch_add(1, Ordering::SeqCst) + 1;
                    peak_prefetch.fetch_max(now, Ordering::SeqCst);
                }
                for _ in 0..lane.below(4) {
                    tokio::task::yield_now().await;
                }
                if priority == Priority::Prefetch {
                    held_prefetch.fetch_sub(1, Ordering::SeqCst);
                }
                held.fetch_sub(1, Ordering::SeqCst);
                drop(permit);
            }));
        }
        for task in tasks {
            timeout(Duration::from_secs(5), task).await.expect("a task never got its slot").unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) <= max, "round {round}: {} permits held with max {max}", peak.load(Ordering::SeqCst));
        assert!(
            peak_prefetch.load(Ordering::SeqCst) <= prefetch_limit,
            "round {round}: {} prefetch permits held with a limit of {prefetch_limit}",
            peak_prefetch.load(Ordering::SeqCst)
        );
        drop(slots.acquire(Priority::Playback).await.unwrap());
        assert_eq!((slots.in_use(), slots.waiting()), (0, 0), "round {round}: slots or waiters leaked");
    }
}

#[tokio::test]
async fn inv_proxy_27_queued_player_requests_are_granted_before_a_waiting_prefetch() {
    for reserved in [0, 1] {
        let slots = PrioritySlots::new(3, reserved, 8);
        let first = slots.acquire(Priority::Playback).await.unwrap();
        let second = slots.acquire(Priority::Playback).await.unwrap();
        let third = slots.acquire(Priority::Playback).await.unwrap();
        let prefetch = tokio::spawn({ let slots = slots.clone(); async move { slots.acquire(Priority::Prefetch).await.unwrap() } });
        wait_for_queue(&slots, 1).await;
        let player = |slots: &PrioritySlots| tokio::spawn({ let slots = slots.clone(); async move { slots.acquire(Priority::Playback).await.unwrap() } });
        let a = player(&slots);
        wait_for_queue(&slots, 2).await;
        let b = player(&slots);
        wait_for_queue(&slots, 3).await;

        drop(first);
        let a = timeout(Duration::from_secs(1), a).await.expect("the first queued player request was not granted").unwrap();
        assert!(!prefetch.is_finished(), "reserved {reserved}: prefetch started while a player request waited");
        drop(second);
        let b = timeout(Duration::from_secs(1), b).await.expect("the second queued player request was not granted").unwrap();
        assert!(!prefetch.is_finished(), "reserved {reserved}: prefetch started while a player request waited");
        drop((third, a, b));
        timeout(Duration::from_secs(1), prefetch).await.expect("prefetch starved").unwrap();
    }
}

#[tokio::test]
async fn inv_proxy_30_a_waiter_whose_sender_vanished_gets_an_error_not_an_endless_wait() {
    let slots = PrioritySlots::new(1, 0, 8);
    let _held = slots.acquire(Priority::Playback).await.unwrap();
    let waiting = tokio::spawn({
        let slots = slots.clone();
        async move { slots.acquire(Priority::Playback).await.map(drop) }
    });
    wait_for_queue(&slots, 1).await;
    slots.lose_waiters();

    let outcome = timeout(Duration::from_secs(1), waiting).await.expect("the waiter hangs after its sender vanished").unwrap();

    assert_eq!(outcome, Err(QueueFull));
}

struct SlowClose {
    gate: Mutex<mpsc::Receiver<()>>,
    closed: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl BodyReader for SlowClose {
    async fn next_chunk(&self) -> Result<Option<Vec<u8>>, MediaError> {
        Ok(None)
    }

    fn release(&self) {
        let _ = self.gate.lock().unwrap().recv();
        self.closed.store(true, Ordering::SeqCst);
    }
}

struct SlowPlatform(Mutex<Option<Arc<SlowClose>>>);

#[async_trait::async_trait]
impl crate::ffi::Fetcher for SlowPlatform {
    async fn fetch(&self, _request: FetchRequest) -> Result<FetchResponse, MediaError> {
        let body = self.0.lock().unwrap().take().unwrap();
        Ok(FetchResponse { status: 200, headers: HashMap::new(), body })
    }
}

#[tokio::test]
async fn inv_proxy_31_a_permit_is_returned_only_after_the_platform_closed_the_response() {
    let (open_gate, gate) = mpsc::channel();
    let closed = Arc::new(AtomicBool::new(false));
    let platform = Arc::new(SlowPlatform(Mutex::new(Some(Arc::new(SlowClose { gate: Mutex::new(gate), closed: closed.clone() })))));
    let slots = PrioritySlots::new(1, 0, 8);
    let permit = slots.acquire(Priority::Playback).await.unwrap();
    let request = crate::fetch::Request { url: "https://o.test/a".to_string(), ..Default::default() };
    let response = ForeignFetcher::new(platform, InputLimits::default()).fetch(request).await.unwrap();

    drop(permit.hold(response.body));
    tokio::time::sleep(SHORT).await;

    assert_eq!(slots.in_use(), 1, "the permit was returned while the platform still held the connection");
    open_gate.send(()).unwrap();
    for _ in 0..200 {
        if slots.in_use() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(closed.load(Ordering::SeqCst));
    assert_eq!(slots.in_use(), 0);
}
