use super::*;
use super::layer_harness::*;
use crate::api::test_support::{assembly, eventually, eventually_async, shared, FakeSettings, MockFactory};
use crate::api::{MediaError, MediaItem};
use crate::fetch::mock::MockFetcher;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;

async fn prepared(events: &mut tokio::sync::broadcast::Receiver<Vec<String>>, url: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if events.recv().await.unwrap().iter().any(|item| item == url) {
                return;
            }
        }
    }).await.expect("page preparation in time");
}

#[derive(Default)]
struct ReadySeen(Mutex<Vec<bool>>);

impl WindowReadyListener for ReadySeen {
    fn changed(&self, ready: bool) {
        self.0.lock().unwrap().push(ready);
    }
}

impl ReadySeen {
    fn is_ready(&self) -> bool {
        self.0.lock().unwrap().last() == Some(&true)
    }
}

/// Serves every url at once except the ones named `slow`, which wait until the test opens them.
#[derive(Default)]
struct Gated {
    open: Mutex<std::collections::HashSet<String>>,
    changed: tokio::sync::Notify,
}

impl Gated {
    fn release(&self, name: &str) {
        self.open.lock().unwrap().insert(format!("https://o.test/{name}.jpg"));
        self.changed.notify_waiters();
    }
}

#[async_trait]
impl Fetcher for Gated {
    async fn fetch(&self, request: super::FetchRequest) -> Result<super::FetchResponse, MediaError> {
        while request.url.contains("slow") {
            let changed = self.changed.notified();
            if self.open.lock().unwrap().contains(&request.url) {
                break;
            }
            changed.await;
        }
        let body = Arc::new(Served(Mutex::new(Some(b"origin bytes".to_vec()))));
        Ok(super::FetchResponse { status: 200, headers: HashMap::from([("Content-Type".to_string(), "image/jpeg".to_string())]), body })
    }
}

fn images(names: &[&str]) -> Vec<MediaItem> {
    names.iter().map(|name| MediaItem::bare(&format!("https://o.test/{name}.jpg"), crate::api::Kind::Image)).collect()
}

#[test]
fn inv_window_01_a_video_window_is_ready_by_the_first_frame_or_the_cache_and_a_page_change_resets_it() {
    let dir = tempfile::tempdir().unwrap();
    let caller = caller();
    let factory: Arc<MockFactory> = Arc::default();
    let (layer, cache) = caller.block_on(async {
        let parts = crate::api::assemble(assembly(dir.path(), Arc::new(MockFetcher::default()), factory.clone())).await.unwrap();
        let cache = parts.cache.clone();
        (MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new()), cache)
    });
    let (framed, bare, cached) = ("https://o.test/framed.mp4", "https://o.test/bare.mp4", "https://o.test/cached.mp4");
    caller.block_on(cache.put("https://o.test/cached.mp4#0", crate::cache::Kind::Stream, b"first block")).unwrap();
    let seen = Arc::new(ReadySeen::default());
    let mut preparation = super::ingress::readiness::prepared();
    let window = ring(&layer, 0, [framed, bare, cached].map(|url| Some(MediaItem::bare_video(url))).to_vec());
    window.subscribe_ready(seen.clone()).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(framed).is_some()));
    caller.block_on(prepared(&mut preparation, framed));
    assert!(!seen.is_ready());

    let (index, slot) = factory.slot_holding(framed).unwrap();
    layer.player_events().visual_ready(index, slot.generation());
    caller.block_on(eventually(|| seen.is_ready()));
    window.move_to(1).unwrap();
    caller.block_on(eventually(|| !seen.is_ready()));
    window.move_to(2).unwrap();

    caller.block_on(eventually(|| seen.is_ready()));
}

#[test]
fn inv_window_07_a_move_between_cached_pages_never_reports_not_ready() {
    let dir = tempfile::tempdir().unwrap();
    let caller = caller();
    let factory: Arc<MockFactory> = Arc::default();
    let (layer, cache) = caller.block_on(async {
        let parts = crate::api::assemble(assembly(dir.path(), Arc::new(MockFetcher::default()), factory.clone())).await.unwrap();
        let cache = parts.cache.clone();
        (MediaLayer::from_parts(None, Handle::current(), parts, None, Vec::new()), cache)
    });
    let (first, second, loading) = ("https://o.test/first.mp4", "https://o.test/second.mp4", "https://o.test/loading.mp4");
    for url in [first, second] {
        caller.block_on(cache.put(&format!("{url}#0"), crate::cache::Kind::Stream, b"first block")).unwrap();
    }
    let seen = Arc::new(ReadySeen::default());
    let mut preparation = super::ingress::readiness::prepared();
    let window = ring(&layer, 0, [first, second, loading].map(|url| Some(MediaItem::bare_video(url))).to_vec());
    window.subscribe_ready(seen.clone()).unwrap();
    caller.block_on(eventually(|| seen.is_ready()));
    window.move_to(1).unwrap();
    caller.block_on(prepared(&mut preparation, second));
    assert_eq!(*seen.0.lock().unwrap(), vec![true], "the report dipped between two cached pages");
    window.move_to(2).unwrap();
    caller.block_on(eventually(|| !seen.is_ready()));
}

#[test]
fn inv_window_02_an_image_window_is_ready_when_every_source_of_the_active_page_is_cached_or_failed_and_a_move_resets_it() {
    let dir = tempfile::tempdir().unwrap();
    let origin = Arc::new(Gated::default());
    let layer = MediaLayer::open(config(dir.path(), origin.clone(), shared::<FakeSettings>(), None)).unwrap();
    let caller = caller();
    let seen = Arc::new(ReadySeen::default());
    let mut preparation = super::ingress::readiness::prepared();
    let grid = layer.open_image_window("grid".to_string(), 1, false).unwrap();
    grid.subscribe_ready(seen.clone()).unwrap();
    grid.replace_rows(vec![images(&["a"])]).unwrap();

    caller.block_on(eventually_async(|| layer.is_cached(images(&["a"]).remove(0))));
    caller.block_on(eventually(|| seen.is_ready()));
    grid.replace_rows(vec![images(&["a", "slow-a"])]).unwrap();
    caller.block_on(prepared(&mut preparation, "https://o.test/slow-a.jpg"));
    assert!(!seen.is_ready(), "ready while a source of the page was still loading");
    origin.release("slow-a");
    caller.block_on(eventually(|| seen.is_ready()));
    grid.replace_rows(vec![images(&["a", "slow-a"]), images(&["slow-b"])]).unwrap();
    grid.show_rows(1, 1).unwrap();
    caller.block_on(eventually(|| !seen.is_ready()));
    origin.release("slow-b");
    caller.block_on(eventually(|| seen.is_ready()));

    let quiet_dir = tempfile::tempdir().unwrap();
    let quiet_origin = Arc::new(Gated::default());
    let quiet = MediaLayer::open(config(quiet_dir.path(), quiet_origin.clone(), shared::<FakeSettings>(), None)).unwrap();
    quiet.player_events().background();
    caller.block_on(eventually_async(|| async { matches!(quiet.controlled(|assembled| async move { assembled.stage.prefetch_suspended() }).await, Ok(true)) }));
    let fetched = Arc::new(ReadySeen::default());
    let late = quiet.open_image_window("grid".to_string(), 1, false).unwrap();
    late.subscribe_ready(fetched.clone()).unwrap();
    late.replace_rows(vec![images(&["slow-c"])]).unwrap();
    quiet_origin.release("slow-c");
    caller.block_on(quiet.fetch_image("https://o.test/slow-c.jpg".to_string(), HashMap::new())).unwrap();
    caller.block_on(eventually(|| fetched.is_ready()));

    let offline_dir = tempfile::tempdir().unwrap();
    let offline = MediaLayer::open(config(offline_dir.path(), Arc::new(Offline), shared::<FakeSettings>(), None)).unwrap();
    let failed = Arc::new(ReadySeen::default());
    let broken = offline.open_image_window("grid".to_string(), 1, false).unwrap();
    broken.subscribe_ready(failed.clone()).unwrap();
    broken.replace_rows(vec![images(&["x"])]).unwrap();
    caller.block_on(eventually(|| failed.is_ready()));
}
