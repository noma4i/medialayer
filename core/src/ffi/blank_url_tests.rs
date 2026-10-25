use super::ingress::PlaceInput;
use super::*;
use crate::api::test_support::{assembly, eventually, shared, MockFactory};
use crate::api::{Kind, MediaError, MediaItem, MediaState, Placeholder};
use crate::fetch::mock::MockFetcher;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

struct Seen(Mutex<Vec<MediaState>>);

impl StateListener for Seen {
    fn changed(&self, state: MediaState) {
        self.0.lock().unwrap().push(state);
    }

    fn failed(&self, _: MediaError) {}
}

fn caller() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

fn layer_over(dir: &std::path::Path, factory: &Arc<MockFactory>, caller: &tokio::runtime::Runtime) -> Arc<MediaLayer> {
    caller.block_on(async {
        let parts = crate::api::assemble(assembly(dir, Arc::new(MockFetcher::default()), factory.clone())).await.unwrap();
        MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new())
    })
}

#[test]
fn inv_source_06_a_place_without_a_url_fails_from_the_core_and_loads_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = shared();
    let caller = caller();
    let layer = layer_over(dir.path(), &factory, &caller);
    let seen = Arc::new(Seen(Mutex::new(Vec::new())));
    let _subscription = layer.subscribe_state("  ".to_string(), seen.clone()).unwrap();

    let place = layer.open_place(MediaItem::bare("  ", Kind::Video)).unwrap();
    place.update(PlaceInput { playing: true, visible: true, ..PlaceInput::default() }).unwrap();

    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Failed { detail: "the source has no url".to_string() })));
    assert!(factory.slot_holding("  ").is_none());

    place.release();
    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Idle)));
}

#[test]
fn inv_source_05_a_resource_url_is_cached_by_definition_and_needs_no_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = shared();
    let caller = caller();
    let layer = layer_over(dir.path(), &factory, &caller);

    for url in ["android.resource://app.test/2131230720", "asset:///intro.mp4"] {
        let item = MediaItem::bare(url, Kind::Video);
        assert!(caller.block_on(layer.is_cached(item.clone())), "{url}");
        assert_eq!(caller.block_on(layer.placeholder(item, false)), Placeholder::None, "{url}");
    }
    assert!(!caller.block_on(layer.is_cached(MediaItem::bare("https://o.test/none.mp4", Kind::Video))));
}
