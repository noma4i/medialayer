use super::*;
use super::layer_harness::*;
use crate::api::test_support::{eventually, shared, slow_assembly, FakeSettings};
use crate::api::{MediaError, MediaItem};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Hanging {
    fetched: Mutex<Vec<String>>,
    released: Arc<std::sync::atomic::AtomicUsize>,
}

struct Endless(Arc<std::sync::atomic::AtomicUsize>);

#[async_trait]
impl BodyReader for Endless {
    async fn next_chunk(&self) -> Result<Option<Vec<u8>>, MediaError> {
        std::future::pending().await
    }

    fn release(&self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait]
impl Fetcher for Hanging {
    async fn fetch(&self, request: super::FetchRequest) -> Result<super::FetchResponse, MediaError> {
        self.fetched.lock().unwrap().push(request.url);
        Ok(super::FetchResponse { status: 200, headers: HashMap::new(), body: Arc::new(Endless(self.released.clone())) })
    }
}

impl Hanging {
    fn fetches(&self, name: &str) -> usize {
        self.fetched.lock().unwrap().iter().filter(|url| url.contains(name)).count()
    }

    fn is_released(&self) -> bool {
        self.released.load(std::sync::atomic::Ordering::SeqCst) > 0
    }
}

fn whole_photo(name: &str) -> MediaItem {
    MediaItem { strategy: crate::prefetch::CacheStrategy::Whole, ..MediaItem::bare(&format!("https://o.test/{name}"), crate::api::Kind::Image) }
}

fn hanging_layer(dir: &std::path::Path) -> (Arc<MediaLayer>, Arc<Hanging>) {
    let origin = Arc::new(Hanging::default());
    (MediaLayer::open(config(dir, origin.clone(), shared::<FakeSettings>(), None)).unwrap(), origin)
}

#[test]
fn inv_strategy_16_a_declaration_made_before_the_assembly_downloads_after_it_without_any_place_or_window() {
    let dir = tempfile::tempdir().unwrap();
    slow_assembly(dir.path(), SLOW);
    let origin = Arc::new(Origin::default());
    let layer = MediaLayer::open(config(dir.path(), origin.clone(), shared::<FakeSettings>(), None)).unwrap();

    let declaration = layer.open_whole_declaration().unwrap();
    let windowed = MediaItem::bare("https://o.test/c.jpg", crate::api::Kind::Image);
    declaration.add(vec![whole_photo("a.jpg"), whole_photo("b.jpg"), windowed]).unwrap();

    caller().block_on(eventually(|| {
        let seen = origin.seen.lock().unwrap();
        ["a.jpg", "b.jpg", "c.jpg"].iter().all(|name| seen.iter().any(|request| request.url.contains(name)))
    }));
}

#[test]
fn inv_strategy_19_a_declaration_ignores_sources_that_are_not_remote() {
    let dir = tempfile::tempdir().unwrap();
    let origin = Arc::new(Origin::default());
    let layer = MediaLayer::open(config(dir.path(), origin.clone(), shared::<FakeSettings>(), None)).unwrap();
    let declaration = layer.open_whole_declaration().unwrap();
    let ignored = ["file:///bundle/local.jpg", "", "ftp://o.test/ftp.jpg", "https://", "o.test/bare.jpg"]
        .map(|url| MediaItem { url: url.to_string(), ..whole_photo("x") });

    declaration.add(ignored.into_iter().chain([whole_photo("remote.jpg")]).collect()).unwrap();

    let caller = caller();
    caller.block_on(eventually(|| origin.seen.lock().unwrap().iter().any(|request| request.url.ends_with("remote.jpg"))));
    caller.block_on(layer.fetch_image("https://o.test/after.jpg".to_string(), HashMap::new())).unwrap();
    let seen: Vec<String> = origin.seen.lock().unwrap().iter().map(|request| request.url.clone()).collect();
    assert_eq!(seen.iter().filter(|url| !url.ends_with("remote.jpg") && !url.ends_with("after.jpg")).count(), 0, "{seen:?}");
}

#[test]
fn inv_strategy_18_closing_the_declaration_cancels_a_load_nobody_else_holds() {
    let dir = tempfile::tempdir().unwrap();
    let (layer, origin) = hanging_layer(dir.path());
    let first = layer.open_whole_declaration().unwrap();
    let second = layer.open_whole_declaration().unwrap();
    first.add(vec![whole_photo("held.jpg")]).unwrap();
    second.add(vec![whole_photo("held.jpg")]).unwrap();
    let caller = caller();
    caller.block_on(eventually(|| origin.fetches("held.jpg") == 1));

    first.release();
    caller.block_on(layer.stats()).unwrap();
    assert!(!origin.is_released(), "a source another declaration holds was cancelled");

    second.release();
    caller.block_on(eventually(|| origin.is_released()));
    assert!(matches!(second.add(vec![whole_photo("late.jpg")]), Err(MediaError::Closed)));
}

fn decode_layer(dir: &std::path::Path, budget: u64) -> Arc<MediaLayer> {
    let mut open = config(dir, Arc::new(Offline), shared::<FakeSettings>(), None);
    open.defaults = Limits { memory_budget: budget, ..default_limits() };
    MediaLayer::open(open).unwrap()
}

#[test]
fn inv_memory_14_a_decode_waits_for_the_ledger_and_a_released_hold_admits_the_next() {
    let dir = tempfile::tempdir().unwrap();
    let layer = decode_layer(dir.path(), 64 << 20);
    let caller = caller();
    caller.block_on(async {
        let first = layer.reserve_decode(40 << 20).await.unwrap();
        assert_eq!(first.bytes(), 40 << 20);
        let mut second = Box::pin(layer.reserve_decode(40 << 20));
        assert!(pending_once(async { let _ = second.as_mut().await; }).await, "a second decode got bytes the ledger did not have");
        first.release();
        assert_eq!(first.bytes(), 0);
        assert_eq!(second.await.unwrap().bytes(), 40 << 20);
    });
}

#[test]
fn inv_memory_14_a_decode_above_the_ceiling_is_refused_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let layer = decode_layer(dir.path(), 64 << 20);
    let refused = caller().block_on(layer.reserve_decode(1 << 30));
    assert!(matches!(refused, Err(MediaError::Overloaded { .. })));
}

#[test]
fn inv_memory_15_the_decode_cost_is_one_rgba_raster_and_refuses_empty_and_oversized_sides() {
    assert_eq!(crate::ffi::decode_cost(100, 50), Some(20_000));
    assert_eq!(crate::ffi::decode_cost(16_384, 16_384), Some(16_384 * 16_384 * 4));
    assert_eq!(crate::ffi::decode_cost(16_385, 1), None);
    assert_eq!(crate::ffi::decode_cost(0, 10), None);
}

#[test]
fn inv_capture_13_a_session_hold_shares_the_budget_with_decodes_and_a_release_admits_the_next() {
    let dir = tempfile::tempdir().unwrap();
    let layer = decode_layer(dir.path(), 64 << 20);
    caller().block_on(async {
        let session = layer.reserve_capture(40 << 20).await.unwrap();
        assert_eq!(session.bytes(), 40 << 20);
        let mut decode = Box::pin(layer.reserve_decode(40 << 20));
        assert!(pending_once(async { let _ = decode.as_mut().await; }).await, "a decode got bytes the capture session holds");
        session.release();
        assert_eq!(decode.await.unwrap().bytes(), 40 << 20);
        assert!(matches!(layer.reserve_capture(1 << 30).await, Err(MediaError::Overloaded { .. })));
    });
}
