use super::memory_store::Failure;
use super::proxy_harness::*;
use super::*;
use crate::api::Kind;
use crate::failures::{FailureListener, FailureStage, Failures, MediaFailure};
use crate::fetch::mock::Script;
use std::sync::Mutex;

const IMAGE: &str = "https://o.test/a.jpg";

#[derive(Default)]
struct Collected(Mutex<Vec<MediaFailure>>);

impl FailureListener for Collected {
    fn failed(&self, failure: MediaFailure) {
        self.0.lock().unwrap().push(failure);
    }

    fn closed(&self) {}
}

async fn watched_rig() -> (Rig, Arc<Collected>, crate::api::Subscription) {
    let failures = Arc::new(Failures::default());
    let collected = Arc::new(Collected::default());
    let subscription = failures.subscribe(collected.clone());
    let mut config = config(8);
    config.failures = failures;
    (rig_with(config).await, collected, subscription)
}

#[tokio::test]
async fn inv_failures_01_a_store_write_failure_still_delivers_the_image_and_reports_once() {
    for failure in [Failure::Begin, Failure::Append(1), Failure::Commit] {
        let (rig, collected, _subscription) = watched_rig().await;
        rig.store.fail_at(Some(failure));
        rig.fetcher.route(IMAGE, Script::ok(pattern(3000, 1)));
        let fetched = within(10, rig.proxy.fetch_bytes(IMAGE, &HashMap::new(), 1 << 20)).await.expect("the image is delivered");
        assert_eq!(fetched.bytes, pattern(3000, 1), "{failure:?}");
        assert_eq!(fetched.source, ImageOrigin::Network, "{failure:?}");
        let reported = collected.0.lock().unwrap().clone();
        assert_eq!(reported.len(), 1, "{failure:?}");
        assert_eq!((reported[0].kind, reported[0].url.as_str(), reported[0].stage), (Kind::Image, IMAGE, FailureStage::Store), "{failure:?}");
    }
}

#[tokio::test]
async fn inv_failures_02_a_network_failure_reports_once() {
    let (rig, collected, _subscription) = watched_rig().await;
    rig.fetcher.set_offline(true);
    let fetched = within(10, rig.proxy.fetch_bytes(IMAGE, &HashMap::new(), 1 << 20)).await;
    assert!(fetched.is_err());
    let reported = collected.0.lock().unwrap().clone();
    assert_eq!(reported.len(), 1);
    assert_eq!((reported[0].kind, reported[0].url.as_str(), reported[0].stage), (Kind::Image, IMAGE, FailureStage::Load));
}

#[tokio::test]
async fn inv_failures_03_a_cancelled_subscription_stops_the_events() {
    let (rig, collected, subscription) = watched_rig().await;
    drop(subscription);
    rig.fetcher.set_offline(true);
    let _ = within(10, rig.proxy.fetch_bytes(IMAGE, &HashMap::new(), 1 << 20)).await;
    assert!(collected.0.lock().unwrap().is_empty());
}
