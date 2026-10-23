use crate::api::signals::Signals;
use crate::api::source::MediaItem;
use crate::api::slots::SlotCall;
use crate::api::test_support::{eventually, rig, shared, url_of, videos, FakeRelease, Rig};
use crate::api::tones::{TonePixels, TonePreparation};
use crate::api::types::Kind;
use crate::fetch::mock::Script;
use crate::prefetch::CacheStrategy;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::runtime::Handle;

const MIB: usize = 1024 * 1024;

fn signals(rig: &Rig) -> Signals {
    let stage = rig.parts.stage.clone();
    Signals::new(
        &Handle::current(),
        Arc::new(stage.clone()),
        Arc::new(rig.parts.prefetcher.clone()),
        Arc::new(stage),
        shared::<FakeRelease>(),
        vec![Arc::new(rig.parts.prefetcher.clone()), Arc::new(rig.parts.tones.clone())],
        rig.parts.ledger.clone(),
        rig.parts.clock.clone(),
    )
}

async fn backgrounded(rig: &Rig, signals: &Signals) {
    signals.background();
    eventually(|| rig.parts.stage.state().ring.is_background()).await;
}

async fn quiet() {
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
}

fn whole(url: &str, kind: Kind) -> MediaItem {
    MediaItem { strategy: CacheStrategy::Whole, ..MediaItem::bare(url, kind) }
}

#[tokio::test]
async fn inv_prefetch_22_a_whole_declared_after_background_waits_for_the_foreground() {
    let rig = rig().await;
    let signals = signals(&rig);
    let clip = "https://o.test/clip.mp4";
    rig.fetcher.route(clip, Script::ok(vec![5; 2 * MIB + MIB / 2]));

    backgrounded(&rig, &signals).await;
    rig.parts.stage.replace_window(0, vec![Some(whole(clip, Kind::Video))]);
    quiet().await;
    assert_eq!(rig.fetcher.calls(), 0, "a whole source started in the background");

    signals.foreground();
    eventually(|| rig.fetcher.calls_for(clip) >= 3).await;
}

#[tokio::test]
async fn inv_prefetch_22_a_window_declared_after_background_waits_for_the_foreground() {
    let rig = rig().await;
    let signals = signals(&rig);
    for index in 0..4 {
        rig.fetcher.route(&url_of(index), Script::ok(vec![5; 100]));
    }

    backgrounded(&rig, &signals).await;
    rig.parts.stage.replace_window(0, videos(4));
    quiet().await;
    assert_eq!(rig.fetcher.calls(), 0, "a window source started in the background");

    signals.foreground();
    eventually(|| rig.fetcher.calls() > 0).await;
}

struct Hanging {
    started: AtomicUsize,
    dropped: AtomicUsize,
}

struct CountDrop<'a>(&'a AtomicUsize);

impl Drop for CountDrop<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl TonePreparation for Hanging {
    async fn prepare(&self, _source: MediaItem) -> Option<TonePixels> {
        self.started.fetch_add(1, Ordering::SeqCst);
        let _guard = CountDrop(&self.dropped);
        std::future::pending::<()>().await;
        None
    }
}

#[tokio::test]
async fn inv_tone_14_background_cancels_tone_preparation_and_the_foreground_restarts_demand() {
    let rig = rig().await;
    let signals = signals(&rig);
    let preparation = Arc::new(Hanging { started: AtomicUsize::new(0), dropped: AtomicUsize::new(0) });
    rig.parts.tones.set_preparation(preparation.clone());
    let first = MediaItem::bare("https://o.test/a.jpg", Kind::Image);
    let second = MediaItem::bare("https://o.test/b.jpg", Kind::Image);
    let _first = rig.parts.tones.subscribe(&first, Arc::new(|_| {}));
    eventually(|| preparation.started.load(Ordering::SeqCst) == 1).await;

    backgrounded(&rig, &signals).await;
    eventually(|| preparation.dropped.load(Ordering::SeqCst) == 1).await;
    let _second = rig.parts.tones.subscribe(&second, Arc::new(|_| {}));
    quiet().await;
    assert_eq!(preparation.started.load(Ordering::SeqCst), 1, "tone work started in the background");

    signals.foreground();
    eventually(|| preparation.started.load(Ordering::SeqCst) == 3).await;
}

async fn within(work: impl FnOnce() + Send + 'static) -> bool {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        work();
        let _ = done.send(());
    });
    tokio::task::spawn_blocking(move || finished.recv_timeout(std::time::Duration::from_secs(3)).is_ok()).await.unwrap()
}

#[test]
fn inv_api_35_a_slot_sample_runs_outside_the_stage_lock() {
    for release in [false, true] {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let finished = runtime.block_on(async move {
            let rig = rig().await;
            let stage = rig.parts.stage.clone();
            let url = url_of(0);
            stage.replace_window(0, videos(3));
            stage.set_playing(&url, true, &[]);
            eventually(|| rig.factory.slot_holding(&url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))).await;
            let (_, slot) = rig.holder(&url);
            slot.sample.lock().unwrap().position_ms = 5_000;
            let reentrant = stage.clone();
            *slot.on_sample.lock().unwrap() = Some(Arc::new(move || reentrant.set_muted_all(true)));
            let trigger = stage.clone();
            within(move || if release { trigger.release_all() } else { trigger.replace_window(0, Vec::new()) }).await
        });
        runtime.shutdown_background();
        assert!(finished, "a reentrant sample blocked the stage (release_all: {release})");
    }
}
