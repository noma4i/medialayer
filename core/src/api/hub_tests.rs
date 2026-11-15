use super::test_support::eventually;
use super::*;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

type Log = Arc<Mutex<Vec<u32>>>;

#[tokio::test]
async fn inv_api_02_unsubscribe_visits_only_the_removed_listener() {
    for count in [1, 64, 1024] {
        let hub = Hub::new(Handle::current());
        let mut subscriptions: Vec<_> = (0..count).map(|_| hub.subscribe(Arc::new(|_: u32| {}))).collect();
        drop(subscriptions.pop());
        assert_eq!(hub.listener_count(), count - 1);
        assert_eq!(hub.unsubscribe_visits(), 1, "listeners={count}");
    }
}

fn recorder() -> (Log, Deliver<u32>) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let sink = log.clone();
    (log, Arc::new(move |value| {
        sink.lock().unwrap().push(value);
        crate::test_signal::fire();
    }))
}

fn last(log: &Log) -> Option<u32> {
    log.lock().unwrap().last().copied()
}

struct Blocked {
    log: Log,
    gate: mpsc::Sender<()>,
    _subscription: Subscription,
}

fn subscribe_blocked(hub: &Hub<u32>) -> Blocked {
    let (gate, wait) = mpsc::channel::<()>();
    let wait = Mutex::new(wait);
    let (log, record) = recorder();
    let deliver: Deliver<u32> = Arc::new(move |value| {
        record(value);
        let _ = wait.lock().unwrap().recv();
    });
    Blocked { log, gate, _subscription: hub.subscribe(deliver) }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv_api_02_burst_behind_a_busy_listener_collapses_to_the_latest_value() {
    let hub = Hub::new(Handle::current());
    let slow = subscribe_blocked(&hub);
    let (fast_log, fast) = recorder();
    let _fast = hub.subscribe(fast);

    hub.publish(0);
    eventually(|| last(&slow.log) == Some(0)).await;
    (1..=99).for_each(|value| hub.publish(value));
    eventually(|| last(&fast_log) == Some(99)).await;
    assert_eq!(last(&slow.log), Some(0), "the fast listener reached 99 while the slow one was still blocked on 0");
    let _ = slow.gate.send(());
    eventually(|| last(&slow.log) == Some(99)).await;
    let _ = slow.gate.send(());

    assert_eq!(*slow.log.lock().unwrap(), vec![0, 99]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inv_api_02_dropping_the_subscription_stops_deliveries() {
    let hub = Hub::new(Handle::current());
    let (dropped_log, dropped) = recorder();
    let (marker_log, marker) = recorder();
    let _marker = hub.subscribe(marker);
    drop(hub.subscribe(dropped));
    assert_eq!(hub.listener_count(), 1);

    hub.publish(7);
    eventually(|| last(&marker_log) == Some(7)).await;

    assert!(dropped_log.lock().unwrap().is_empty());
}
