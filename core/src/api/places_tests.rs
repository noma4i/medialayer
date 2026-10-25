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
async fn inv_api_03_a_loading_place_becomes_ready_once_and_never_flickers_back_to_loading() {
    let places = Places::new(Handle::current());
    let (seen, _subscription) = watch(&places, "v1");
    eventually(|| seen.lock().unwrap().len() == 1).await;

    places.declared("v1", false);
    eventually(|| seen.lock().unwrap().len() == 2).await;
    places.ready("v1");
    eventually(|| seen.lock().unwrap().len() == 3).await;
    places.declared("v1", false);
    assert_eq!(places.state("v1"), MediaState::Ready);
    places.ready("v1");
    places.failed("v1", "end");
    eventually(|| seen.lock().unwrap().len() == 4).await;

    assert_eq!(
        *seen.lock().unwrap(),
        vec![MediaState::Idle, MediaState::Loading, MediaState::Ready, MediaState::Failed { detail: "end".to_string() }]
    );
}

#[tokio::test]
async fn inv_placeholder_03_a_cached_video_or_a_ready_slot_skips_loading() {
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
async fn inv_api_03_a_failed_place_reports_the_reason_and_loads_again_when_declared_again() {
    let places = Places::new(Handle::current());

    places.declared("v1", false);
    places.failed("v1", "HTTP 404");
    assert_eq!(places.state("v1"), MediaState::Failed { detail: "HTTP 404".to_string() });
    places.declared("v1", false);

    assert_eq!(places.state("v1"), MediaState::Loading);
}

#[tokio::test]
async fn inv_api_03_an_unloaded_slot_sends_a_declared_place_back_to_loading_and_forgets_an_undeclared_one() {
    let places = Places::new(Handle::current());
    places.declared("seen", false);
    places.ready("seen");
    places.ready("neighbour");

    places.unloaded("seen");
    places.unloaded("neighbour");

    assert_eq!((places.state("seen"), places.state("neighbour")), (MediaState::Loading, MediaState::Idle));
}

#[tokio::test]
async fn inv_api_03_a_native_loader_moves_a_place_through_loading_ready_and_failed_in_order() {
    let places = Places::new(Handle::current());
    let (seen, _subscription) = watch(&places, "image");
    eventually(|| seen.lock().unwrap().len() == 1).await;
    let failure = MediaState::Failed { detail: "decode".to_string() };

    let steps = [
        (MediaState::Loading, true),
        (MediaState::Loading, false),
        (MediaState::Ready, true),
        (MediaState::Loading, false),
        (failure.clone(), true),
        (MediaState::Loading, true),
        (MediaState::Ready, true),
    ];
    for (state, publishes) in steps {
        let before = places.state("image");
        places.item_state("image", state.clone());
        if publishes {
            eventually(|| seen.lock().unwrap().last() == Some(&state)).await;
        } else {
            assert_eq!(places.state("image"), before);
        }
    }

    assert_eq!(
        *seen.lock().unwrap(),
        vec![MediaState::Idle, MediaState::Loading, MediaState::Ready, failure, MediaState::Loading, MediaState::Ready]
    );
}

#[tokio::test]
async fn inv_api_13_a_rendered_or_ended_place_is_never_pulled_back_by_a_late_ready_loading_or_declaration() {
    for settled in [MediaState::Rendered, MediaState::Ended] {
        let places = Places::new(Handle::current());
        places.declared("v1", false);
        places.ready("v1");
        places.rendered("v1");
        if settled == MediaState::Ended {
            places.ended("v1");
        }

        places.ready("v1");
        places.item_state("v1", MediaState::Loading);
        places.item_state("v1", MediaState::Ready);
        places.declared("v1", false);
        places.declared("v1", true);

        assert_eq!(places.state("v1"), settled);
        assert!(settled.is_ready());
    }
    let places = Places::new(Handle::current());
    places.declared("v2", false);
    places.ended("v2");
    places.resumed("v2");
    assert_eq!(places.state("v2"), MediaState::Loading);
}

#[tokio::test]
async fn inv_api_13_a_place_ends_once_from_a_ready_state_and_a_repeated_end_is_ignored() {
    let places = Places::new(Handle::current());
    places.declared("v1", false);

    let mut steps = vec![(places.ended("v1"), places.state("v1"))];
    places.ready("v1");
    steps.push((places.ended("v1"), places.state("v1")));
    steps.push((places.ended("v1"), places.state("v1")));
    places.rendered("v1");
    steps.push((false, places.state("v1")));
    places.resumed("v1");
    steps.push((false, places.state("v1")));

    assert_eq!(
        steps,
        vec![
            (false, MediaState::Loading),
            (true, MediaState::Ended),
            (false, MediaState::Ended),
            (false, MediaState::Ended),
            (false, MediaState::Rendered)
        ]
    );
}
