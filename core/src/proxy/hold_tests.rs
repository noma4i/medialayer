use super::memory_store::MemoryStore;
use super::*;
use crate::fetch::mock::MockFetcher;
use crate::hls::Playlist;
use crate::pool::Retention;
use std::sync::atomic::AtomicUsize;

const MEDIA: &str = "https://o.test/clip/media.m3u8";
const HOLD: Duration = Duration::from_secs(60);

struct Idle {
    probes: AtomicUsize,
}

impl Playhead for Idle {
    fn position_ms(&self, _playlist: &str) -> PositionProbe {
        self.probes.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Some(0) })
    }

    fn level(&self, _playlist: &str) -> Option<Retention> {
        Some(Retention::Hot)
    }

    fn readahead(&self, _playlist: &str, _hint_ms: u64) {}
}

fn segment(index: usize) -> String {
    format!("https://cdn.test/clip/s{index}.ts")
}

#[tokio::test(start_paused = true)]
async fn inv_proxy_37_a_held_request_wakes_by_events_and_one_ceiling_not_by_polling() {
    let config = Config { hold_ceiling: HOLD, ..Config::default() };
    let proxy = Proxy::start(config, Arc::new(MockFetcher::new()), Arc::new(MemoryStore::new())).await.unwrap();
    let playhead = Arc::new(Idle { probes: AtomicUsize::new(0) });
    proxy.set_playhead(playhead.clone());
    let mut text = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:4\n");
    for index in 0..40 {
        text.push_str(&format!("#EXTINF:3.000,\n{}\n", segment(index)));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    proxy.inner.windows.register_plan(MEDIA, &Playlist::parse(&text, MEDIA));

    let held = tokio::spawn({
        let inner = proxy.inner.clone();
        let target = segment(30);
        async move { inner.hold_for_playhead(&target, &target).await }
    });
    for _ in 0..(HOLD.as_secs() - 1) {
        tokio::time::advance(Duration::from_secs(1)).await;
    }
    assert!(!held.is_finished(), "the request was released before the ceiling");
    tokio::time::advance(Duration::from_secs(2)).await;
    held.await.unwrap();

    let probes = playhead.probes.load(Ordering::SeqCst);
    assert!(probes <= 2, "the held request probed the player {probes} times in {HOLD:?}");
}
