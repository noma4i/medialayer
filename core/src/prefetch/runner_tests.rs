use super::mock_loader::{Fetch, MockLoader};
use super::*;
use crate::cache::{Clock, ManualClock};
use crate::limits::Limits;
use crate::proxy::Priority;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::{error::TryRecvError, Receiver};

fn url(index: usize) -> String {
    format!("https://o.test/m{index}.jpg")
}

fn sources(count: usize) -> Vec<Option<Source>> {
    (0..count)
        .map(|index| Some(Source { url: url(index), headers: HashMap::new(), kind: SourceKind::Image, strategy: CacheStrategy::Window }))
        .collect()
}

fn image(url: &str) -> Source {
    Source { url: url.to_string(), headers: HashMap::new(), kind: SourceKind::Image, strategy: CacheStrategy::Window }
}

fn pages_of(prefix: &str, count: usize) -> Vec<Vec<Source>> {
    (0..count).map(|index| vec![image(&format!("https://o.test/{prefix}{index}a.jpg")), image(&format!("https://o.test/{prefix}{index}b.jpg"))]).collect()
}

fn page_urls(prefix: &str, pages: &[usize]) -> HashSet<String> {
    pages.iter().flat_map(|page| ["a", "b"].map(|side| format!("https://o.test/{prefix}{page}{side}.jpg"))).collect()
}

pub(super) fn video(url: &str) -> Source {
    Source { url: url.to_string(), headers: HashMap::new(), kind: SourceKind::Video, strategy: CacheStrategy::Window }
}

pub(super) struct Rig {
    pub(super) prefetcher: Prefetcher,
    pub(super) loader: Arc<MockLoader>,
    pub(super) clock: Arc<ManualClock>,
}

pub(super) fn rig_with(tweak: impl FnOnce(&mut Limits)) -> Rig {
    let mut limits = Limits { max_concurrent: 8, prefetch_from: 2, prefetch_to: 5, ..Limits::default() };
    tweak(&mut limits);
    let loader = Arc::new(MockLoader::default());
    let clock = Arc::new(ManualClock::new(1_000));
    let ledger = PrefetchLedger::new(Duration::from_secs(limits.dedupe_window_seconds.into()), limits.dedupe_max_entries as usize);
    let prefetcher = Prefetcher::new(loader.clone(), &limits, ledger, clock.clone() as Arc<dyn Clock>);
    Rig { prefetcher, loader, clock }
}

pub(super) fn rig() -> Rig {
    rig_with(|_| {})
}

#[track_caller]
pub(super) fn eventually(mut condition: impl FnMut() -> bool) -> impl std::future::Future<Output = ()> {
    let caller = std::panic::Location::caller();
    async move { tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let seen = crate::test_signal::current();
            if condition() { return; }
            crate::test_signal::changed_since(seen).await;
        }
    }).await.unwrap_or_else(|_| panic!("condition at {caller} never became true")); }
}

pub(super) async fn next(events: &mut Receiver<Settled>) -> Settled {
    tokio::time::timeout(Duration::from_secs(5), events.recv()).await.expect("event in time").expect("channel open")
}

pub(super) async fn wait_calls(loader: &MockLoader, count: usize) {
    while loader.calls().len() < count {
        loader.started().await;
    }
}

pub(super) async fn quiet() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
}

fn names(urls: &[String]) -> HashSet<String> {
    urls.iter().cloned().collect()
}

#[tokio::test]
async fn inv_prefetch_03_shifting_the_window_starts_the_new_page_and_cancels_the_departed_one() {
    let rig = rig();
    let mut events = rig.prefetcher.settled();
    rig.loader.hang_all();
    rig.prefetcher.set_window(0, sources(20));
    wait_calls(&rig.loader, 4).await;
    assert_eq!(names(&rig.loader.urls()), names(&[url(2), url(3), url(4), url(5)]));

    rig.prefetcher.set_window(1, sources(20));
    assert_eq!(next(&mut events).await, Settled { url: url(2), outcome: Outcome::Cancelled });
    wait_calls(&rig.loader, 5).await;

    assert_eq!(rig.loader.urls()[4], url(6));
    assert_eq!(rig.loader.active(), 4);
    assert_eq!(rig.prefetcher.counters().cancelled, 1);
}

#[tokio::test]
async fn inv_prefetch_01_a_source_loaded_inside_the_dedupe_window_is_not_loaded_again_until_the_window_passes() {
    let rig = rig();
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.set_window(0, sources(20));
    for _ in 0..4 {
        next(&mut events).await;
    }
    rig.prefetcher.set_window(1, sources(20));
    next(&mut events).await;
    assert_eq!(rig.loader.urls().len(), 5);

    rig.prefetcher.set_window(0, sources(20));
    assert_eq!(rig.loader.urls().len(), 5);
    assert_eq!(rig.prefetcher.counters().deduped, 1);

    rig.clock.advance_ms(61_000);
    rig.prefetcher.set_window(1, sources(20));
    next(&mut events).await;
    rig.prefetcher.set_window(0, sources(20));
    next(&mut events).await;
    assert_eq!(rig.loader.urls()[5..], [url(6), url(2)]);
}

#[tokio::test]
async fn inv_prefetch_01_a_failed_source_is_loaded_again_when_it_re_enters_the_window() {
    let rig = rig();
    let mut events = rig.prefetcher.settled();
    rig.loader.fail(&url(2), true);
    rig.prefetcher.set_window(0, sources(20));
    let mut outcomes = HashMap::new();
    for _ in 0..4 {
        let settled = next(&mut events).await;
        outcomes.insert(settled.url, settled.outcome);
    }
    assert_eq!(outcomes[&url(2)], Outcome::Failed(format!("{} refused", url(2))));
    assert_eq!(outcomes[&url(3)], Outcome::Loaded);

    rig.loader.fail(&url(2), false);
    rig.prefetcher.set_window(1, sources(20));
    rig.prefetcher.set_window(0, sources(20));
    let outcomes: HashMap<_, _> = [next(&mut events).await, next(&mut events).await].into_iter().map(|event| (event.url, event.outcome)).collect();
    assert_eq!(outcomes[&url(2)], Outcome::Loaded);
    assert_eq!(rig.prefetcher.counters().failed, 1);
}

#[tokio::test]
async fn inv_prefetch_01_a_failed_source_is_loaded_again_when_the_same_window_is_declared_again() {
    let rig = rig();
    let mut events = rig.prefetcher.settled();
    rig.loader.fail(&url(2), true);
    rig.prefetcher.set_window(0, sources(20));
    let mut outcomes = HashMap::new();
    for _ in 0..4 {
        let settled = next(&mut events).await;
        outcomes.insert(settled.url, settled.outcome);
    }
    assert_eq!(outcomes[&url(2)], Outcome::Failed(format!("{} refused", url(2))));

    rig.loader.fail(&url(2), false);
    rig.prefetcher.set_window(0, sources(20));

    assert_eq!(next(&mut events).await, Settled { url: url(2), outcome: Outcome::Loaded });
}

#[tokio::test]
async fn inv_prefetch_03_each_started_source_settles_exactly_once() {
    let rig = rig();
    let mut events = rig.prefetcher.settled();
    for _ in 0..3 {
        rig.prefetcher.set_window(0, sources(20));
    }
    let mut seen = Vec::new();
    for _ in 0..4 {
        seen.push(next(&mut events).await);
    }
    rig.prefetcher.wait_finished(4).await;

    assert!(seen.iter().all(|settled| settled.outcome == Outcome::Loaded));
    assert_eq!(seen.iter().map(|settled| settled.url.clone()).collect::<HashSet<_>>().len(), 4);
    assert_eq!(events.try_recv().unwrap_err(), TryRecvError::Empty);
}

#[tokio::test]
async fn inv_prefetch_06_hls_sources_load_the_first_variant_and_the_init_before_the_chunk_of_their_distance() {
    let master = "https://o.test/hls/master.m3u8";
    let multivariant = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=900\nv0/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=300\nv1/index.m3u8\n";
    let mapped = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2.0,\ns0.m4s\n#EXTINF:2.0,\ns1.m4s\n#EXTINF:2.0,\ns2.m4s\n";
    let rig = rig_with(|limits| limits.leading_chunks = crate::limits::LeadingChunks::new(&[1]).unwrap());
    rig.loader.serve(master, multivariant);
    rig.loader.serve("https://o.test/hls/v0/index.m3u8", mapped);
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.set_window(0, vec![None, None, Some(video(master))]);
    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);

    assert_eq!(rig.loader.urls(), [master, "https://o.test/hls/v0/index.m3u8", "https://o.test/hls/v0/init.mp4", "https://o.test/hls/v0/s0.m4s"]);
    assert!(rig.loader.calls().iter().all(|call| call.priority == Priority::Prefetch && call.fetch == Fetch::Whole));
}

#[tokio::test]
async fn inv_prefetch_06_images_load_whole_while_audio_and_plain_video_load_only_their_first_block_and_no_playlist_is_read() {
    let rig = rig();
    let feed = vec![
        None,
        None,
        Some(Source { url: "https://o.test/a.jpg".to_string(), headers: HashMap::new(), kind: SourceKind::Image, strategy: CacheStrategy::Window }),
        Some(Source { url: "https://o.test/b.m4a".to_string(), headers: HashMap::new(), kind: SourceKind::Audio, strategy: CacheStrategy::Window }),
        Some(video("https://o.test/c.mp4")),
    ];
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.set_window(0, feed);
    for _ in 0..3 {
        next(&mut events).await;
    }

    let fetched: HashMap<String, Fetch> = rig.loader.calls().into_iter().map(|call| (call.url, call.fetch)).collect();
    assert_eq!(fetched["https://o.test/a.jpg"], Fetch::Whole);
    assert_eq!(fetched["https://o.test/b.m4a"], Fetch::FirstBlock);
    assert_eq!(fetched["https://o.test/c.mp4"], Fetch::FirstBlock);
    assert_eq!(rig.loader.stored_reads(), 0);
}

#[tokio::test(start_paused = true)]
async fn inv_prefetch_03_a_hung_load_settles_as_timed_out_after_the_configured_timeout() {
    let rig = rig();
    let mut events = rig.prefetcher.settled();
    rig.loader.hang_all();
    rig.prefetcher.set_window(0, sources(20));
    wait_calls(&rig.loader, 4).await;

    tokio::time::advance(Duration::from_secs(29)).await;
    assert_eq!(events.try_recv().unwrap_err(), TryRecvError::Empty);
    tokio::time::advance(Duration::from_secs(2)).await;
    for _ in 0..4 {
        assert_eq!(next(&mut events).await.outcome, Outcome::TimedOut);
    }
    assert_eq!((rig.prefetcher.counters().failed, rig.prefetcher.live_tasks(), rig.loader.active()), (4, 0, 0));
}

#[tokio::test]
async fn inv_prefetch_04_stop_all_cancels_every_load_and_forgets_the_window_so_the_same_feed_starts_again() {
    let rig = rig();
    let mut events = rig.prefetcher.settled();
    rig.loader.hang_all();
    rig.prefetcher.set_window(0, sources(20));
    eventually(|| rig.loader.active() == 4).await;

    rig.prefetcher.stop_all();
    for _ in 0..4 {
        assert_eq!(next(&mut events).await.outcome, Outcome::Cancelled);
    }
    assert_eq!((rig.loader.active(), rig.prefetcher.live_tasks(), rig.prefetcher.counters().cancelled), (0, 0, 4));

    rig.prefetcher.set_window(0, sources(20));
    eventually(|| rig.loader.urls().len() == 8).await;
}

#[tokio::test]
async fn inv_prefetch_04_resume_starts_the_last_window_again_without_declaring_it() {
    let rig = rig();
    let mut events = rig.prefetcher.settled();
    rig.loader.hang_all();
    rig.prefetcher.set_window(0, sources(20));
    eventually(|| rig.loader.active() == 4).await;
    rig.prefetcher.stop_all();
    for _ in 0..4 {
        next(&mut events).await;
    }

    rig.prefetcher.resume();

    eventually(|| rig.loader.urls().len() == 8).await;
}

#[tokio::test]
async fn inv_prefetch_02_no_more_than_max_concurrent_loads_run_and_cancel_hands_the_slot_to_the_next_source() {
    let rig = rig_with(|limits| limits.max_concurrent = 2);
    rig.loader.hang_all();
    rig.prefetcher.set_window(0, sources(20));
    wait_calls(&rig.loader, 2).await;
    rig.prefetcher.wait_queued(2).await;
    assert_eq!(names(&rig.loader.urls()), names(&[url(2), url(3)]));

    rig.prefetcher.cancel(&url(2));
    wait_calls(&rig.loader, 3).await;
    assert_eq!(rig.loader.urls()[2], url(4));
    assert_eq!((rig.loader.active(), rig.loader.peak()), (2, 2));
}

#[tokio::test]
async fn inv_prefetch_03_a_thousand_window_shifts_leave_no_live_tasks_behind() {
    let rig = rig();
    rig.loader.hang_all();
    let feed = sources(1200);
    let mut most_live = 0;
    for active in 0..1000 {
        rig.prefetcher.set_window(active, feed.clone());
        tokio::task::yield_now().await;
        most_live = most_live.max(rig.prefetcher.live_tasks());
    }
    rig.prefetcher.wait_live_at_most(4).await;
    assert_eq!(rig.prefetcher.live_tasks(), 4);
    assert!(most_live <= 6, "most live tasks {most_live}");

    rig.prefetcher.stop_all();
    eventually(|| rig.prefetcher.live_tasks() == 0 && rig.loader.active() == 0).await;
}

#[tokio::test]
async fn inv_prefetch_02_set_limits_changes_the_window_and_the_parallelism_for_the_next_jobs() {
    let rig = rig_with(|limits| {
        limits.max_concurrent = 8;
        limits.prefetch_from = 1;
        limits.prefetch_to = 1;
    });
    rig.loader.hang_all();

    rig.prefetcher.set_limits(&Limits { max_concurrent: 1, prefetch_from: 1, prefetch_to: 3, ..Limits::default() });
    rig.prefetcher.set_window(0, sources(6));

    wait_calls(&rig.loader, 1).await;
    rig.prefetcher.wait_queued(2).await;
    assert_eq!(rig.prefetcher.counters().requested, 3);
    assert_eq!((rig.loader.active(), rig.loader.peak()), (1, 1));
}

#[tokio::test]
async fn inv_prefetch_08_named_windows_start_and_cancel_only_their_own_sources_and_leave_the_video_window_alone() {
    let rig = rig_with(|limits| limits.max_concurrent = 32);
    let mut events = rig.prefetcher.settled();
    rig.loader.hang_all();
    rig.prefetcher.set_image_windows(HashMap::from([("grid".to_string(), PageWindow { behind: 0, from: 0, to: 2 }), ("posters".to_string(), PageWindow { behind: 0, from: 0, to: 2 })]));
    rig.prefetcher.set_window(0, sources(20));
    rig.prefetcher.set_image_window("grid", 0, pages_of("g", 20));
    rig.prefetcher.set_image_window("posters", 0, pages_of("p", 20));
    wait_calls(&rig.loader, 12).await;
    assert_eq!(rig.prefetcher.counters().cancelled, 0);

    rig.prefetcher.set_image_window("grid", 10, pages_of("g", 20));
    for _ in 0..4 {
        assert_eq!(next(&mut events).await.outcome, Outcome::Cancelled);
    }
    wait_calls(&rig.loader, 16).await;

    assert_eq!(rig.prefetcher.counters().cancelled, 4);
    assert_eq!(rig.loader.active(), 12);
    let started: HashSet<String> = rig.loader.urls().into_iter().collect();
    assert!(page_urls("g", &[10, 11]).is_subset(&started));
    assert!(page_urls("p", &[0, 1]).is_subset(&started));
    assert!((2..=5).all(|index| started.contains(&url(index))));
}

#[tokio::test]
async fn inv_prefetch_08_a_source_wanted_by_two_windows_loads_once_and_is_cancelled_only_when_no_window_wants_it() {
    let rig = rig();
    let mut events = rig.prefetcher.settled();
    rig.loader.hang_all();
    let shared_page = vec![vec![image("https://o.test/shared.jpg")]];
    rig.prefetcher.set_image_window("one", 0, shared_page.clone());
    rig.prefetcher.set_image_window("two", 0, shared_page);
    wait_calls(&rig.loader, 1).await;

    rig.prefetcher.set_image_window("one", 0, Vec::<Vec<Source>>::new());
    assert_eq!((rig.loader.urls().len(), rig.loader.active(), rig.prefetcher.counters().cancelled), (1, 1, 0));

    rig.prefetcher.set_image_window("two", 0, Vec::<Vec<Source>>::new());
    assert_eq!(next(&mut events).await, Settled { url: "https://o.test/shared.jpg".to_string(), outcome: Outcome::Cancelled });
    assert_eq!((rig.loader.urls().len(), rig.prefetcher.counters().cancelled), (1, 1));
}

#[tokio::test]
async fn inv_prefetch_10_a_window_takes_the_range_of_its_name_and_an_unknown_name_takes_the_default() {
    let rig = rig();
    rig.prefetcher.set_image_windows(HashMap::from([("avatar".to_string(), PageWindow { behind: 1, from: 1, to: 3 })]));
    let mut events = rig.prefetcher.settled();

    rig.prefetcher.set_image_window("avatar", 5, pages_of("a", 20));
    for _ in 0..6 {
        next(&mut events).await;
    }
    assert_eq!(rig.loader.urls().into_iter().collect::<HashSet<_>>(), page_urls("a", &[6, 7, 4]));

    rig.prefetcher.set_image_window("other", 5, pages_of("x", 20));
    for _ in 0..22 {
        next(&mut events).await;
    }
    assert_eq!(rig.loader.urls().len(), 6 + 22);
    assert_eq!(rig.loader.urls().into_iter().filter(|url| url.starts_with("https://o.test/x")).collect::<HashSet<_>>(), page_urls("x", &[3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]));
}
