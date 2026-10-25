use super::Stage;
use crate::api::ports::PoolControl;
use crate::api::signals::Signals;
use crate::api::source::MediaItem;
use crate::api::slots::{PlayerSlot, SlotCall};
use crate::api::test_support::{eventually, rig, shared, url_of, videos, FakeRelease, Rig};
use crate::api::tones::{TonePixels, TonePreparation};
use crate::api::types::Kind;
use crate::fetch::mock::Script;
use crate::pool::Pressure;
use crate::prefetch::CacheStrategy;
use std::collections::{BTreeMap, HashMap};
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

type Trigger = fn(&Stage, &str, u32);

#[test]
fn inv_api_35_a_slot_sample_runs_outside_the_stage_lock() {
    let entries: [(&str, bool, Trigger); 6] = [
        ("set_playing", false, |stage, url, _| stage.set_playing(url, true, &[])),
        ("seek_fraction", false, |stage, url, _| stage.seek_fraction(url, 0.5)),
        ("ready", false, |stage, _, slot| stage.ready(slot as usize, stage.generation_at(slot as usize))),
        ("visual_ready", false, |stage, _, slot| stage.visual_ready(slot as usize, stage.generation_at(slot as usize))),
        ("release_under", false, |stage, _, _| drop(stage.release_under(0, Pressure::Critical))),
        ("replace_window", true, |stage, _, _| stage.replace_window(0, Vec::new())),
    ];
    for (entry, playing, trigger) in entries {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let (finished, sampled) = runtime.block_on(async move {
            let rig = rig().await;
            let stage = rig.parts.stage.clone();
            let url = url_of(0);
            stage.replace_window(0, videos(3));
            if playing {
                stage.set_playing(&url, true, &[]);
                eventually(|| rig.factory.slot_holding(&url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))).await;
            }
            eventually(|| rig.factory.slot_holding(&url).is_some_and(|(_, slot)| slot.loads() > 0)).await;
            let (index, slot) = rig.holder(&url);
            {
                let mut sample = slot.sample.lock().unwrap();
                sample.position_ms = 5_000;
                sample.duration_ms = Some(10_000);
            }
            let hooked = Arc::new(AtomicUsize::new(0));
            let (reentrant, counted) = (stage.clone(), hooked.clone());
            *slot.on_sample.lock().unwrap() = Some(Arc::new(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                reentrant.set_muted_all(true);
            }));
            let before = hooked.load(Ordering::SeqCst);
            let target = stage.clone();
            let finished = within(move || trigger(&target, &url, index)).await;
            (finished, hooked.load(Ordering::SeqCst) > before)
        });
        runtime.shutdown_background();
        assert!(finished, "a reentrant sample blocked the stage (entry: {entry})");
        assert!(sampled, "the entry never sampled the slot, the check proves nothing (entry: {entry})");
    }
}

fn tagged(url: &str, kind: Kind) -> MediaItem {
    MediaItem { headers: HashMap::from([("X-Owner".to_string(), url.to_string())]), strategy: CacheStrategy::OnDemand, ..MediaItem::bare(url, kind) }
}

fn assert_catalog(stage: &Stage, step: &str, owned: &[&MediaItem]) {
    let state = stage.state();
    let mut expected: BTreeMap<String, MediaItem> = owned.iter().map(|item| (item.url.clone(), (*item).clone())).collect();
    for (key, loaded) in &state.loaded {
        expected.entry(key.clone()).or_insert_with(|| loaded.source.clone());
    }
    let actual: BTreeMap<String, MediaItem> = state.catalog.iter().map(|(key, item)| (key.clone(), item.clone())).collect();
    assert_eq!(actual, expected, "{step}");
}

#[tokio::test]
async fn inv_api_40_hidden_images_leave_the_catalog() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let mut sizes = Vec::new();
    let mut next = 0;
    for series in [1usize, 32, 256] {
        for _ in 0..series {
            let item = MediaItem::bare(&format!("https://o.test/hidden-{next}.jpg"), Kind::Image);
            next += 1;
            stage.visible(item.clone(), true);
            stage.visible(item, false);
            stage.inner.declaration_completed.notified().await;
        }
        let state = stage.state();
        sizes.push((series, state.catalog.len(), state.declarations.len()));
    }
    assert!(sizes.iter().all(|(_, catalog, _)| *catalog == 0), "hidden images stay in the catalog (series, catalog, declarations): {sizes:?}");
}

#[tokio::test]
async fn inv_api_40_a_source_stays_while_any_owner_lives_and_leaves_with_the_last() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let clip = tagged(&url_of(5), Kind::Video);
    let photo = tagged("https://o.test/owned.jpg", Kind::Image);
    let unloaded = || stage.state().loaded.is_empty();

    stage.visible(clip.clone(), true);
    assert_catalog(stage, "declared", &[&clip]);
    stage.attach(&clip.url, 7);
    stage.visible(clip.clone(), false);
    eventually(unloaded).await;
    assert_catalog(stage, "hidden, held by the view", &[&clip]);
    stage.set_playing(&clip.url, true, &[]);
    eventually(|| stage.state().loaded.contains_key(&clip.url)).await;
    stage.detach(&clip.url);
    assert_catalog(stage, "detached, held by the playing intent and the slot", &[&clip]);
    stage.set_playing(&clip.url, false, &[]);
    stage.release_under(rig.parts.clock.now_ms(), Pressure::Critical);
    eventually(unloaded).await;
    assert_catalog(stage, "last owner gone", &[]);

    stage.replace_window(0, vec![Some(clip.clone())]);
    assert_catalog(stage, "in the window", &[&clip]);
    stage.visible(photo.clone(), true);
    stage.replace_window(0, Vec::new());
    stage.release_under(rig.parts.clock.now_ms(), Pressure::Critical);
    eventually(unloaded).await;
    assert_catalog(stage, "window gone, the declaration stays", &[&photo]);
    stage.visible(photo, false);
    assert_catalog(stage, "everything gone", &[]);
}

fn seek_hints(stage: &Stage) -> usize {
    stage.inner.seeks.lock().unwrap().len()
}

#[tokio::test]
async fn inv_api_41_release_all_forgets_seek_hints() {
    let rig = rig().await;
    let stage = rig.parts.stage.clone();
    let mut sizes = Vec::new();
    for index in 0..32 {
        let url = format!("https://o.test/hint{index}.mp4");
        stage.replace_window(0, vec![Some(crate::api::test_support::video(&url))]);
        eventually(|| rig.factory.slot_holding(&url).is_some_and(|(_, slot)| slot.loads() > 0)).await;
        stage.seek(&url, 1_000 + index as u64);
        stage.release_all();
        sizes.push(seek_hints(&stage));
    }
    assert!(sizes.iter().all(|size| *size == 0), "seek hints survived release_all: {sizes:?}");
}

#[tokio::test]
async fn inv_api_41_a_seek_hint_leaves_with_its_unloaded_source() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let url = url_of(0);
    stage.replace_window(0, videos(1));
    eventually(|| stage.state().loaded.contains_key(&url)).await;
    stage.seek(&url, 5_000);
    assert_eq!(seek_hints(stage), 1, "a seek on a loaded source leaves a hint");

    stage.release_under(rig.parts.clock.now_ms(), Pressure::Critical);
    eventually(|| stage.state().loaded.is_empty()).await;

    assert_eq!(seek_hints(stage), 0, "the hint outlived its source");
}

fn claim_trigger_mark(owner: &std::sync::Mutex<Option<std::thread::ThreadId>>) -> bool {
    let mut mark = owner.lock().unwrap();
    let is_trigger = *mark == Some(std::thread::current().id());
    if is_trigger {
        mark.take();
    }
    is_trigger
}

#[test]
fn inv_api_35_a_sample_that_returns_after_a_reload_of_the_same_key_is_not_applied() {
    let entries: [(&str, Trigger); 2] = [
        ("ready", |stage, _, slot| stage.ready(slot as usize, stage.generation_at(slot as usize))),
        ("seek_fraction", |stage, url, _| stage.seek_fraction(url, 0.5)),
    ];
    let mut problems = Vec::new();
    for (entry, trigger) in entries {
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let (finished, seeks, kept) = runtime.block_on(async move {
            let rig = rig().await;
            let stage = rig.parts.stage.clone();
            let url = url_of(0);
            stage.replace_window(0, videos(3));
            eventually(|| rig.factory.slot_holding(&url).is_some_and(|(_, slot)| slot.loads() > 0)).await;
            let (index, slot) = rig.holder(&url);
            slot.sample.lock().unwrap().duration_ms = None;
            if entry == "ready" {
                stage.seek_fraction(&url, 0.5);
            }
            slot.sample.lock().unwrap().duration_ms = Some(10_000);
            let triggered = Arc::new(std::sync::Mutex::new(None));
            let sampled_by = Arc::new(std::sync::Mutex::new(Vec::new()));
            let foreign = Arc::new(std::sync::Mutex::new(None));
            let (reloading, key, owner, seen) = (stage.clone(), url.clone(), triggered.clone(), sampled_by.clone());
            *slot.on_sample.lock().unwrap() = Some(Arc::new(move || {
                seen.lock().unwrap().push(std::thread::current().id());
                if claim_trigger_mark(&owner) {
                    reloading.reload(&key);
                }
            }));
            let (target, key, sampler, foreign_id) = (stage.clone(), url.clone(), slot.clone(), foreign.clone());
            let finished = within(move || {
                *triggered.lock().unwrap() = Some(std::thread::current().id());
                *foreign_id.lock().unwrap() = Some(std::thread::spawn(move || {
                    sampler.sample();
                    std::thread::current().id()
                }).join().unwrap());
                trigger(&target, &key, index)
            })
            .await;
            let seeks: Vec<SlotCall> = slot.calls().into_iter().filter(|call| matches!(call, SlotCall::Seek { .. })).collect();
            let kept = stage.state().positions.take_fraction(&url).is_some();
            let foreign_sampled = foreign.lock().unwrap().is_some_and(|id| sampled_by.lock().unwrap().contains(&id));
            (finished && foreign_sampled, seeks, kept)
        });
        runtime.shutdown_background();
        if !finished || !seeks.is_empty() || !kept {
            problems.push(format!("{entry}: returned and foreign sample seen {finished}, seeks {seeks:?}, fraction kept {kept}"));
        }
    }
    assert!(problems.is_empty(), "a sample of the previous load was applied to the reloaded key: {problems:?}");
}

#[test]
fn inv_api_36_a_parked_sample_that_returns_after_a_release_all_leaves_no_position() {
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let (finished, kept) = runtime.block_on(async move {
        let rig = rig().await;
        let stage = rig.parts.stage.clone();
        let urls: Vec<String> = (0..3).map(url_of).collect();
        stage.replace_window(0, videos(3));
        eventually(|| rig.factory.slot_holding(&urls[0]).is_some_and(|(_, slot)| slot.loads() > 0)).await;
        let (_, slot) = rig.holder(&urls[0]);
        slot.sample.lock().unwrap().position_ms = 5_000;
        let triggered = Arc::new(std::sync::Mutex::new(None));
        let sampled_by = Arc::new(std::sync::Mutex::new(Vec::new()));
        let foreign = Arc::new(std::sync::Mutex::new(None));
        let (releasing, owner, seen) = (stage.clone(), triggered.clone(), sampled_by.clone());
        *slot.on_sample.lock().unwrap() = Some(Arc::new(move || {
            seen.lock().unwrap().push(std::thread::current().id());
            if claim_trigger_mark(&owner) {
                releasing.release_all();
            }
        }));
        let (target, sampler, foreign_id) = (stage.clone(), slot.clone(), foreign.clone());
        let finished = within(move || {
            *triggered.lock().unwrap() = Some(std::thread::current().id());
            *foreign_id.lock().unwrap() = Some(std::thread::spawn(move || {
                sampler.sample();
                std::thread::current().id()
            }).join().unwrap());
            drop(target.release_under(0, Pressure::Critical));
        })
        .await;
        let kept: Vec<_> = urls.iter().filter_map(|url| stage.state().positions.take(url).map(|position| (url.clone(), position))).collect();
        let foreign_sampled = foreign.lock().unwrap().is_some_and(|id| sampled_by.lock().unwrap().contains(&id));
        (finished && foreign_sampled, kept)
    });
    runtime.shutdown_background();
    assert!(finished, "the parked sample never returned or the foreign sample never reached the hook");
    assert!(kept.is_empty(), "a release_all during the parked sample was undone: {kept:?}");
}

#[tokio::test]
async fn inv_api_36_release_all_leaves_no_parked_position() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let urls: Vec<String> = (0..3).map(url_of).collect();
    stage.replace_window(0, videos(3));
    eventually(|| rig.factory.slot_holding(&urls[0]).is_some_and(|(_, slot)| slot.loads() > 0)).await;
    rig.holder(&urls[0]).1.sample.lock().unwrap().position_ms = 5_000;

    stage.release_all();

    assert!(stage.state().loaded.is_empty());
    let kept: Vec<_> = urls.iter().filter_map(|url| stage.state().positions.take(url).map(|position| (url.clone(), position))).collect();
    assert!(kept.is_empty(), "release_all left parked positions: {kept:?}");
}
