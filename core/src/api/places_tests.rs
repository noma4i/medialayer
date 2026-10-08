use super::test_support::eventually;
use super::*;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

fn watch(places: &Places, key: &str) -> (Arc<Mutex<Vec<MediaState>>>, Subscription) {
    let seen: Arc<Mutex<Vec<MediaState>>> = Arc::default();
    let sink = seen.clone();
    let subscription = places.subscribe(key, Arc::new(move |state| sink.lock().unwrap().push(state)));
    (seen, subscription)
}

#[tokio::test]
async fn a_loading_place_becomes_ready_once_and_never_flickers_back_to_loading() {
    let places = Places::new(Handle::current());
    let (seen, _subscription) = watch(&places, "v1");
    eventually(|| seen.lock().unwrap().len() == 1).await;

    places.declared("v1", false);
    eventually(|| seen.lock().unwrap().len() == 2).await;
    places.ready("v1");
    eventually(|| seen.lock().unwrap().len() == 3).await;
    places.declared("v1", false);
    places.ready("v1");
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    assert_eq!(*seen.lock().unwrap(), vec![MediaState::Idle, MediaState::Loading, MediaState::Ready]);
}

#[tokio::test]
async fn a_cached_video_or_a_ready_slot_skips_loading() {
    for ready_first in [false, true] {
        let places = Places::new(Handle::current());
        let (seen, _subscription) = watch(&places, "v1");
        eventually(|| seen.lock().unwrap().len() == 1).await;

        if ready_first {
            places.ready("v1");
            places.declared("v1", false);
        } else {
            places.declared("v1", true);
        }

        eventually(|| seen.lock().unwrap().last() == Some(&MediaState::Ready)).await;
        assert!(!seen.lock().unwrap().contains(&MediaState::Loading), "ready_first={ready_first}");
    }
}

#[tokio::test]
async fn a_failed_place_reports_the_reason_and_loads_again_when_declared_again() {
    let places = Places::new(Handle::current());

    places.declared("v1", false);
    places.failed("v1", "HTTP 404");
    assert_eq!(places.state("v1"), MediaState::Failed { detail: "HTTP 404".to_string() });
    places.declared("v1", false);

    assert_eq!(places.state("v1"), MediaState::Loading);
}

#[tokio::test]
async fn an_unloaded_slot_sends_a_declared_place_back_to_loading_and_forgets_an_undeclared_one() {
    let places = Places::new(Handle::current());
    places.declared("seen", false);
    places.ready("seen");
    places.ready("neighbour");

    places.unloaded("seen");
    places.unloaded("neighbour");

    assert_eq!((places.state("seen"), places.state("neighbour")), (MediaState::Loading, MediaState::Idle));
}
