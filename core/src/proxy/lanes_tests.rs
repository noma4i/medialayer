use super::memory_store::MemoryStore;
use super::test_client::get;
use super::*;
use crate::fetch::mock::{MockFetcher, Script};
use std::time::{Duration, Instant};

const IMAGES: usize = 6;
const IMAGE_DELAY: Duration = Duration::from_millis(400);
const VIDEO_BUDGET: Duration = Duration::from_millis(250);
const PLAYLIST: &[u8] = b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\nseg0.ts\n#EXT-X-ENDLIST\n";

async fn rig(max_concurrent: usize) -> (Proxy, Arc<MockFetcher>) {
    let fetcher = Arc::new(MockFetcher::new());
    let mut config = Config::default();
    config.max_concurrent = max_concurrent;
    config.policy = config.policy.with_timeout(Duration::from_secs(5));
    let proxy = Proxy::start(config, fetcher.clone(), Arc::new(MemoryStore::new())).await.expect("proxy start");
    (proxy, fetcher)
}

fn image(index: usize) -> String {
    format!("https://o.test/image{index}.jpg")
}

fn spawn_images(proxy: &Proxy, fetcher: &MockFetcher) -> Vec<tokio::task::JoinHandle<()>> {
    (0..IMAGES)
        .map(|index| {
            fetcher.route(&image(index), Script::ok(vec![7; 100]).head_delay(IMAGE_DELAY));
            let proxy = proxy.clone();
            tokio::spawn(async move {
                let _ = proxy.fetch_bytes(&image(index), &HashMap::new(), 1 << 20).await;
            })
        })
        .collect()
}

async fn video_wait(proxy: &Proxy, fetcher: &MockFetcher) -> Duration {
    fetcher.route("https://o.test/video.m3u8", Script::ok(PLAYLIST.to_vec()));
    let url = proxy.url_for("https://o.test/video.m3u8", &HashMap::new()).expect("proxy url");
    let parsed = url::Url::parse(&url).unwrap();
    let target = format!("{}?{}", parsed.path(), parsed.query().unwrap_or_default());
    let started = Instant::now();
    let reply = get(proxy.port(), &target).await;
    assert_eq!(reply.status, 200);
    started.elapsed()
}

#[tokio::test]
async fn inv_proxy_20_a_burst_of_images_never_delays_a_video_request() {
    let (proxy, fetcher) = rig(2).await;
    let images = spawn_images(&proxy, &fetcher);
    assert!(super::proxy_harness::eventually(|| proxy.inner.image_slots.in_use() == 2).await);
    let waited = video_wait(&proxy, &fetcher).await;
    assert!(waited < VIDEO_BUDGET, "the video playlist waited {waited:?} behind {IMAGES} images");
    for task in images {
        task.abort();
    }
}
