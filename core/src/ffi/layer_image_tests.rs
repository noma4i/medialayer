use super::*;
use super::layer_harness::*;
use super::ingress::PlaceInput;
use crate::api::test_support::{assembly, eventually, shared, FakeSettings, MockFactory};
use crate::api::{MediaError, MediaItem, MediaState, PlayerSlotFactory, SlotCall};
use crate::fetch::mock::MockFetcher;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

fn origin_layer(dir: &std::path::Path) -> (Arc<MediaLayer>, Arc<Origin>) {
    let origin = Arc::new(Origin::default());
    (MediaLayer::open(config(dir, origin.clone(), shared::<FakeSettings>(), None)).unwrap(), origin)
}

#[test]
fn inv_ffi_03_an_image_loads_through_the_fetcher_into_the_cache_once_and_repeats_come_from_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let (layer, origin) = origin_layer(dir.path());
    let headers = HashMap::from([("Authorization".to_string(), "token".to_string())]);
    let caller = caller();

    let (first, second, together) = caller.block_on(async {
        let first = layer.fetch_image("https://o.test/a.jpg".to_string(), headers.clone()).await;
        let second = layer.fetch_image("https://o.test/a.jpg".to_string(), headers.clone()).await;
        let together = tokio::join!(
            layer.fetch_image("https://o.test/b.jpg".to_string(), HashMap::new()),
            layer.fetch_image("https://o.test/b.jpg".to_string(), HashMap::new())
        );
        (first, second, together)
    });

    assert_eq!((first.unwrap().bytes, second.unwrap().bytes), (b"origin bytes".to_vec(), b"origin bytes".to_vec()));
    assert_eq!((together.0.unwrap().bytes, together.1.unwrap().bytes), (b"origin bytes".to_vec(), b"origin bytes".to_vec()));
    let seen = origin.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].headers.get("Authorization").map(String::as_str), Some("token"));
    assert!(seen.iter().all(|request| request.range_start.is_none()));
}

#[test]
fn inv_ffi_03_an_image_that_cannot_be_fetched_is_a_fetch_error() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), None)).unwrap();

    let result = caller().block_on(layer.fetch_image("https://o.test/a.jpg".to_string(), HashMap::new()));

    assert!(matches!(result, Err(MediaError::Fetch { .. })));
}

#[test]
fn inv_placeholder_09_a_cached_video_keeps_its_cover_until_the_first_frame_is_shown() {
    let dir = tempfile::tempdir().unwrap();
    let caller = caller();
    let (layer, cache) = caller.block_on(async {
        let parts = crate::api::assemble(assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>())).await.unwrap();
        let cache = parts.cache.clone();
        (MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new()), cache)
    });
    let cached = MediaItem { cover: Some("https://o.test/cover.jpg".to_string()), ..MediaItem::bare_video("https://o.test/cached.mp4") };
    let cover = crate::api::Placeholder::Cover { url: "https://o.test/cover.jpg".to_string() };

    caller.block_on(cache.put("https://o.test/cached.mp4#0", crate::cache::Kind::Stream, b"first block")).unwrap();

    assert!(caller.block_on(layer.is_cached(cached.clone())));
    assert_eq!(caller.block_on(layer.placeholder(cached.clone(), false)), cover);
    assert_eq!(caller.block_on(layer.placeholder(cached, true)), crate::api::Placeholder::None);
}

#[test]
fn inv_placeholder_08_no_placeholder_reads_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let caller = caller();
    let (layer, routes) = caller.block_on(async {
        let parts = crate::api::assemble(assembly(dir.path(), Arc::new(MockFetcher::default()), shared::<MockFactory>())).await.unwrap();
        let routes = parts.routes.clone();
        (MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new()), routes)
    });
    let item = |url: &str, kind: crate::api::Kind| MediaItem::bare(url, kind);
    let cases = [
        ("image", item("https://o.test/a.jpg", crate::api::Kind::Image), false),
        ("ready image", item("https://o.test/b.jpg", crate::api::Kind::Image), true),
        ("audio", item("https://o.test/a.mp3", crate::api::Kind::Audio), false),
        ("ready video", item("https://o.test/a.mp4", crate::api::Kind::Video), true),
        ("unready video", item("https://o.test/b.mp4", crate::api::Kind::Video), false),
    ];
    for (name, source, ready) in cases {
        let before = routes.storage_lookups();
        caller.block_on(layer.placeholder(source, ready));
        assert_eq!(routes.storage_lookups() - before, 0, "{name}");
    }
}

#[test]
fn inv_ffi_03_an_item_state_from_a_native_loader_reaches_the_state_subscribers_and_a_seek_reaches_the_player() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::open(config(dir.path(), Arc::new(Offline), shared::<FakeSettings>(), Some(slots))).unwrap();
    let seen = Arc::new(Seen(Mutex::new(Vec::new())));
    let _subscription = layer.subscribe_state("https://o.test/a.jpg".to_string(), seen.clone()).unwrap();
    let caller = caller();
    let image = layer.open_place(MediaItem::bare("https://o.test/a.jpg", crate::api::Kind::Image)).unwrap();

    image.item_state(MediaState::Loading).unwrap();
    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Loading)));
    image.item_state(MediaState::Ready).unwrap();
    caller.block_on(eventually(|| seen.0.lock().unwrap().last() == Some(&MediaState::Ready)));

    let url = "https://o.test/v1.mp4";
    let _ring = ring(&layer, 0, vec![Some(MediaItem::bare_video(url))]);
    let video = place(&layer, url, PlaceInput { view_token: Some(5), ..PlaceInput::default() });
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    video.seek(0.025).unwrap();
    let (_, player) = factory.slot_holding(url).unwrap();
    caller.block_on(eventually(|| player.calls().contains(&SlotCall::Seek { position_ms: 1_500 }) && player.calls().contains(&SlotCall::Attach { view_token: 5 })));
}

#[test]
fn inv_ffi_05_open_takes_the_spec_defaults_without_writing_them_and_a_saved_limit_wins_over_them() {
    let spec = crate::spec::MediaSpec {
        cache: crate::spec::CacheSpec { budget: 3_000_000_000, ..Default::default() },
        videos: vec![crate::spec::VideoSpec { pool: crate::spec::PoolSpec { slots: 5, ..Default::default() }, ..crate::spec::VideoSpec::new("clip") }],
        ..Default::default()
    };
    let parsed = crate::ffi::spec_defaults(spec.to_json()).unwrap();
    let settings: Arc<FakeSettings> = shared();
    let caller = caller();
    let from_spec = |dir: &std::path::Path| {
        let mut open = config(dir, Arc::new(Offline), settings.clone(), None);
        open.defaults = parsed.defaults.clone();
        open.pool = parsed.pool;
        MediaLayer::open(open).unwrap()
    };
    let first_dir = tempfile::tempdir().unwrap();

    let layer = from_spec(first_dir.path());
    let limits = caller.block_on(layer.limits()).unwrap();

    assert_eq!((limits.budget, limits.slots), (3_000_000_000, 5));
    assert!(settings.writes.lock().unwrap().is_empty());
    let saved = Limits { budget: 600_000_000, ..parsed.defaults.clone() };
    caller.block_on(layer.set_limits(saved.clone())).unwrap();
    drop(layer);
    let second_dir = tempfile::tempdir().unwrap();
    assert_eq!(caller.block_on(from_spec(second_dir.path()).limits()).unwrap(), saved);
}

#[test]
fn inv_ffi_05_namespace_headers_given_to_open_give_each_header_value_its_own_cache_entry() {
    let caller = caller();
    let fetch_twice = |namespace: Vec<String>| {
        let dir = tempfile::tempdir().unwrap();
        let origin = Arc::new(Origin::default());
        let mut open = config(dir.path(), origin.clone(), shared::<FakeSettings>(), None);
        open.namespace_headers = namespace;
        let layer = MediaLayer::open(open).unwrap();
        for token in ["one", "two"] {
            let headers = HashMap::from([("Authorization".to_string(), token.to_string())]);
            caller.block_on(layer.fetch_image("https://o.test/a.jpg".to_string(), headers)).unwrap();
        }
        let seen = origin.seen.lock().unwrap().len();
        seen
    };

    assert_eq!((fetch_twice(Vec::new()), fetch_twice(vec!["Authorization".to_string()])), (1, 2));
}

fn poster_pages(count: usize) -> Vec<Vec<MediaItem>> {
    (0..count).map(|index| vec![MediaItem::bare(&format!("https://o.test/p{index}.jpg"), crate::api::Kind::Image)]).collect()
}

#[test]
fn inv_ffi_06_an_image_prefetched_by_a_window_is_a_cache_hit_for_fetch_image() {
    let dir = tempfile::tempdir().unwrap();
    let (layer, origin) = origin_layer(dir.path());
    let caller = caller();

    let grid = layer.open_image_window("grid".to_string(), 1, false).unwrap();
    grid.replace_pages(0, poster_pages(1)).unwrap();
    caller.block_on(eventually(|| !origin.seen.lock().unwrap().is_empty()));
    let fetched = caller.block_on(layer.fetch_image("https://o.test/p0.jpg".to_string(), HashMap::new()));

    assert_eq!(fetched.unwrap().bytes, b"origin bytes".to_vec());
    assert_eq!(origin.seen.lock().unwrap().len(), 1);
}

#[test]
fn inv_cache_35_a_cached_file_the_disk_refuses_to_remove_fails_clear_and_is_one_store_failure() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (layer, _origin) = origin_layer(dir.path());
    let reported: Arc<Reported> = shared();
    let _subscription = layer.subscribe_failures(reported.clone());
    let url = "https://o.test/kept.jpg";
    let caller = caller();
    caller.block_on(layer.fetch_image(url.to_string(), HashMap::new())).unwrap();
    assert!(reported.seen().is_empty(), "{:?}", reported.seen());
    let writable = std::fs::metadata(dir.path()).unwrap().permissions();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();

    let cleared = caller.block_on(layer.clear(None));
    let kept = caller.block_on(layer.is_cached(MediaItem::bare(url, crate::api::Kind::Image)));
    std::fs::set_permissions(dir.path(), writable).unwrap();

    assert!(matches!(cleared, Err(MediaError::Unexpected { .. })), "clear reported success for a file it could not remove: {cleared:?}");
    let failures: Vec<_> = reported.seen().into_iter().filter(|failure| failure.url == url).collect();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].stage, crate::failures::FailureStage::Store, "{failures:?}");
    assert!(kept, "the file left on disk dropped out of the cache");
    caller.block_on(layer.await_closed());
}
