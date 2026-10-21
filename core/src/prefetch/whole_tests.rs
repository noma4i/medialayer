use super::mock_loader::Fetch;
use super::runner_tests::{eventually, next, rig, Rig};
use super::*;
use crate::cache::{classify_kind, Cache, CacheStore, SystemClock};
use crate::fetch::mock::{MockFetcher, Script};
use crate::limits::Limits;
use crate::proxy::{ByteStore, Config, Priority, Proxy};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const MIB: usize = 1024 * 1024;
const MASTER: &str = "https://o.test/hls/master.m3u8";
const MEDIA: &str = "https://o.test/hls/v0/index.m3u8";
const INIT: &str = "https://o.test/hls/v0/init.mp4";
const SEGMENT_0: &str = "https://o.test/hls/v0/s0.m4s";
const SEGMENT_1: &str = "https://o.test/hls/v0/s1.m4s";

fn source(url: &str, kind: SourceKind, strategy: CacheStrategy) -> Source {
    Source { url: url.to_string(), headers: HashMap::new(), kind, strategy }
}

fn whole(url: &str, kind: SourceKind) -> Source {
    source(url, kind, CacheStrategy::Whole)
}

#[tokio::test]
async fn inv_strategy_01_window_then_whole_loads_the_remaining_bytes() {
    let rig = super::runner_tests::rig_with(|limits| {
        limits.prefetch_from = 0;
        limits.prefetch_to = 0;
    });
    let clip = "https://o.test/clip.mp4";
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.set_window(0, vec![Some(source(clip, SourceKind::Video, CacheStrategy::Window))]);
    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));
    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    assert_eq!(rig.loader.calls().iter().map(|call| call.fetch).collect::<Vec<_>>(), [Fetch::FirstBlock, Fetch::AllBlocks]);
}

#[tokio::test]
async fn inv_strategy_06_whole_above_budget_never_reports_complete() {
    let stack = stack().await;
    stack.cache.set_limits(Limits { budget: (2 * MIB) as u64, min_budget: 1, ..Limits::default() }).await.unwrap();
    let url = "https://o.test/large.mp4";
    stack.fetcher.route(url, Script::ok(vec![7; 4 * MIB]));
    let result = ProxyLoader::new(stack.proxy.clone()).load_all_blocks(url, &HashMap::new(), Priority::Prefetch, &tokio::sync::Notify::new()).await;
    assert!(result.is_err(), "incomplete source reported success: {result:?}");
}

#[tokio::test]
async fn inv_prefetch_01_whole_admission_is_bounded_by_the_entry_limit() {
    let rig = super::runner_tests::rig_with(|limits| limits.dedupe_max_entries = 4);
    rig.loader.hang_all();
    for index in 0..100 {
        rig.prefetcher.keep_whole(whole(&format!("https://o.test/{index}.mp4"), SourceKind::Video));
    }
    assert!(rig.prefetcher.live_tasks() <= 4, "{} tasks for a limit of 4", rig.prefetcher.live_tasks());
}

#[tokio::test]
async fn inv_strategy_06_whole_hls_above_budget_never_reports_complete() {
    let stack = stack().await;
    stack.cache.set_limits(Limits { budget: (2 * MIB) as u64, min_budget: 1, ..Limits::default() }).await.unwrap();
    stack.fetcher.route(MASTER, Script::ok(b"#EXTM3U\n#EXTINF:2,\na.ts\n#EXTINF:2,\nb.ts\n#EXTINF:2,\nc.ts\n#EXT-X-ENDLIST\n".to_vec()));
    for name in ["a.ts", "b.ts", "c.ts"] {
        stack.fetcher.route(&format!("https://o.test/hls/{name}"), Script::ok(vec![7; MIB]));
    }
    let result = super::whole::load_whole(&ProxyLoader::new(stack.proxy.clone()), &whole(MASTER, SourceKind::Video), Duration::from_secs(5)).await;
    assert!(matches!(result, Outcome::Failed(_)), "incomplete HLS reported success: {result:?}");
}

#[tokio::test]
async fn inv_strategy_02_running_window_promotes_to_whole_and_survives_window_departure() {
    let rig = super::runner_tests::rig_with(|limits| {
        limits.prefetch_from = 0;
        limits.prefetch_to = 0;
    });
    let clip = "https://o.test/clip.mp4";
    rig.loader.hang(clip);
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.set_window(0, vec![Some(source(clip, SourceKind::Video, CacheStrategy::Window))]);
    rig.loader.started().await;
    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));
    rig.prefetcher.set_window(0, vec![]);
    rig.loader.release(clip);
    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    assert_eq!(rig.loader.calls().iter().map(|call| call.fetch).collect::<Vec<_>>(), [Fetch::FirstBlock, Fetch::AllBlocks]);
}

#[tokio::test]
async fn inv_strategy_12_reset_cancels_work_before_a_new_declaration() {
    let rig = rig();
    let clip = "https://o.test/clip.mp4";
    let mut events = rig.prefetcher.settled();
    rig.loader.hang(clip);
    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));
    rig.loader.started().await;
    rig.prefetcher.forget_loaded();
    assert_eq!(next(&mut events).await.outcome, Outcome::Cancelled);
    rig.loader.release(clip);
    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));
    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
}

#[tokio::test]
async fn inv_strategy_01_a_whole_source_loads_in_full_without_a_window_by_its_kind() {
    let cases = [
        ("https://o.test/clip.mp4", SourceKind::Video, Fetch::AllBlocks, Priority::Prefetch),
        ("https://o.test/circle.mp4", SourceKind::Video, Fetch::AllBlocks, Priority::Prefetch),
        ("https://o.test/voice.m4a", SourceKind::Audio, Fetch::AllBlocks, Priority::Prefetch),
        ("https://o.test/photo.jpg", SourceKind::Image, Fetch::Whole, Priority::ImagePrefetch),
    ];
    for (url, kind, fetch, priority) in cases {
        let rig = rig();
        let mut events = rig.prefetcher.settled();

        rig.prefetcher.keep_whole(whole(url, kind));

        assert_eq!(next(&mut events).await, Settled { url: url.to_string(), outcome: Outcome::Loaded });
        let calls = rig.loader.calls();
        assert_eq!((calls.len(), calls[0].fetch, calls[0].priority), (1, fetch, priority), "{url}");
        assert_eq!(rig.loader.kept(), vec![url.to_string()], "{url}");
    }
}

#[tokio::test]
async fn inv_strategy_01_a_whole_hls_source_loads_the_first_variant_with_every_segment() {
    let rig = rig();
    rig.loader.serve(MASTER, "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=900\nv0/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=100\nv1/index.m3u8\n");
    rig.loader.serve(MEDIA, "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2.0,\ns0.m4s\n#EXTINF:2.0,\ns1.m4s\n#EXTINF:2.0,\ns2.m4s\n#EXT-X-ENDLIST\n");
    let mut events = rig.prefetcher.settled();

    rig.prefetcher.keep_whole(whole(MASTER, SourceKind::Video));

    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    let segments = ["s0.m4s", "s1.m4s", "s2.m4s"].map(|name| format!("https://o.test/hls/v0/{name}"));
    let urls = rig.loader.urls();
    assert_eq!(urls, [MASTER, MEDIA, INIT, &segments[0], &segments[1], &segments[2]].map(str::to_string));
    assert!(rig.loader.calls().iter().all(|call| call.fetch == Fetch::Whole));
}

#[tokio::test]
async fn inv_strategy_01_a_window_or_on_demand_source_is_not_a_whole_load() {
    for strategy in [CacheStrategy::Window, CacheStrategy::OnDemand] {
        let rig = rig();

        rig.prefetcher.keep_whole(source("https://o.test/clip.mp4", SourceKind::Video, strategy));
        super::runner_tests::quiet().await;

        assert!(rig.loader.calls().is_empty(), "{strategy:?}");
    }
}

#[tokio::test]
async fn inv_strategy_02_moving_the_window_and_foreground_play_never_cancel_a_whole_load() {
    let rig = rig();
    let clip = "https://o.test/clip.mp4";
    rig.loader.hang(clip);

    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));
    eventually(|| rig.loader.alive() == [clip]).await;
    for active in [0, 7, 3] {
        rig.prefetcher.set_window(active, vec![None; 12]);
    }
    rig.prefetcher.set_foreground_busy(true);
    rig.prefetcher.set_window(5, vec![None; 12]);
    super::runner_tests::quiet().await;

    assert_eq!(rig.loader.alive(), [clip]);
    assert_eq!(rig.prefetcher.counters().cancelled, 0);
}

#[tokio::test]
async fn inv_strategy_02_a_whole_source_is_not_evicted_by_the_window_budget() {
    let rig = whole_hls_rig();
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.keep_whole(whole(MASTER, SourceKind::Video));
    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);

    let feed = (0..4).map(|item| Some(super::runner_tests::video(&format!("https://o.test/w{item}/master.m3u8")))).collect::<Vec<_>>();
    for item in 0..4 {
        rig.loader.serve(&format!("https://o.test/w{item}/master.m3u8"), "#EXTM3U\n#EXTINF:3,\ns0.ts\n#EXTINF:3,\ns1.ts\n");
    }
    rig.loader.size("https://o.test/w1/s0.ts", 700 * 1024);
    rig.loader.size("https://o.test/w2/s0.ts", 700 * 1024);
    rig.loader.size("https://o.test/w3/s0.ts", 700 * 1024);
    rig.prefetcher.set_window(0, feed);
    eventually(|| !rig.loader.evicted().is_empty()).await;

    assert!(rig.loader.evicted().iter().all(|url| !url.starts_with("https://o.test/hls/")), "{:?}", rig.loader.evicted());
}

fn whole_hls_rig() -> Rig {
    let rig = super::runner_tests::rig_with(|limits| {
        limits.prefetch_from = 1;
        limits.prefetch_to = 3;
        limits.prefetch_budget_mib = 1;
        limits.leading_chunks = crate::limits::LeadingChunks::new(&[1]).unwrap();
    });
    rig.loader.serve(MASTER, "#EXTM3U\n#EXTINF:2.0,\ns0.m4s\n#EXTINF:2.0,\ns1.m4s\n");
    rig
}

#[tokio::test]
async fn inv_strategy_03_declaring_a_source_again_loads_it_once() {
    let rig = rig();
    let clip = "https://o.test/clip.mp4";
    let mut events = rig.prefetcher.settled();

    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));
    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));
    next(&mut events).await;
    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));
    super::runner_tests::quiet().await;

    assert_eq!(rig.loader.urls(), [clip]);
    assert_eq!(rig.prefetcher.counters().deduped, 2);
}

#[tokio::test]
async fn inv_strategy_04_whole_media_loads_one_at_a_time_and_whole_images_two() {
    let rig = rig();
    rig.loader.hang_all();
    for index in 0..3 {
        rig.prefetcher.keep_whole(whole(&format!("https://o.test/v{index}.mp4"), SourceKind::Video));
    }
    eventually(|| rig.loader.active() == 1).await;
    super::runner_tests::quiet().await;
    assert_eq!((rig.loader.active(), rig.loader.peak()), (1, 1));
    rig.prefetcher.forget_windows();

    let rig = super::runner_tests::rig();
    rig.loader.hang_all();
    for index in 0..4 {
        rig.prefetcher.keep_whole(whole(&format!("https://o.test/p{index}.jpg"), SourceKind::Image));
    }
    eventually(|| rig.loader.active() == 2).await;
    super::runner_tests::quiet().await;
    assert_eq!((rig.loader.active(), rig.loader.peak()), (2, 2));
}

#[tokio::test]
async fn inv_strategy_12_a_whole_source_whose_files_were_purged_loads_again_at_the_next_declaration() {
    let rig = rig();
    let clip = "https://o.test/clip.mp4";
    let mut events = rig.prefetcher.settled();
    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));
    next(&mut events).await;

    rig.prefetcher.forget_loaded();
    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));

    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    assert_eq!(rig.loader.urls(), [clip, clip]);
}

#[tokio::test(start_paused = true)]
async fn inv_strategy_05_a_failed_block_is_tried_three_times_then_the_load_stops_and_resumes_on_the_next_declaration() {
    let rig = rig();
    let clip = "https://o.test/clip.mp4";
    let mut events = rig.prefetcher.settled();
    rig.loader.fail(clip, true);

    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));
    let failed = next(&mut events).await;

    assert!(matches!(failed.outcome, Outcome::Failed(_)), "{failed:?}");
    assert_eq!(rig.loader.urls().len(), 3);
    assert_eq!(rig.prefetcher.counters().failed, 1);

    rig.loader.fail(clip, false);
    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));

    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    assert_eq!(rig.loader.urls().len(), 4);
}

#[tokio::test(start_paused = true)]
async fn inv_strategy_05_a_failed_whole_load_starts_again_when_the_app_returns_to_the_foreground() {
    let rig = rig();
    let clip = "https://o.test/clip.mp4";
    let mut events = rig.prefetcher.settled();
    rig.loader.fail(clip, true);
    rig.prefetcher.keep_whole(whole(clip, SourceKind::Video));
    assert!(matches!(next(&mut events).await.outcome, Outcome::Failed(_)));

    rig.loader.fail(clip, false);
    rig.prefetcher.set_foreground_busy(true);
    rig.prefetcher.set_foreground_busy(false);

    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
}

struct Stack {
    dir: tempfile::TempDir,
    cache: Cache,
    fetcher: Arc<MockFetcher>,
    proxy: Proxy,
}

async fn stack_in(dir: tempfile::TempDir) -> Stack {
    let cache = Cache::open(dir.path(), Limits::default(), u64::MAX);
    cache.ready().await;
    let store: Arc<dyn ByteStore> = Arc::new(CacheStore::new(cache.clone(), Arc::new(classify_kind)));
    let fetcher = Arc::new(MockFetcher::new());
    let proxy = Proxy::start(Config::default(), fetcher.clone(), store).await.unwrap();
    Stack { dir, cache, fetcher, proxy }
}

async fn stack() -> Stack {
    stack_in(tempfile::tempdir().unwrap()).await
}

fn ranges(fetcher: &MockFetcher) -> Vec<Option<(u64, Option<u64>)>> {
    fetcher.requests().iter().map(|request| request.range).collect()
}

fn block_range(block: usize, last: usize) -> Option<(u64, Option<u64>)> {
    let size = MIB as u64;
    Some((block as u64 * size, Some(if block == last { 3 * size + size / 2 - 1 } else { (block as u64 + 1) * size - 1 })))
}

#[tokio::test]
async fn inv_strategy_06_a_whole_mp4_is_stored_as_every_block_up_to_its_total_length() {
    let stack = stack().await;
    let url = "https://o.test/clip.mp4";
    stack.fetcher.route(url, Script::ok(vec![7; 3 * MIB + MIB / 2]));
    let loader = ProxyLoader::new(stack.proxy.clone());

    let bytes = loader.load_all_blocks(url, &HashMap::new(), Priority::Prefetch, &tokio::sync::Notify::new()).await.unwrap();

    assert_eq!(bytes, (3 * MIB + MIB / 2) as u64);
    assert_eq!(ranges(&stack.fetcher), (0..4).map(|block| block_range(block, 3)).collect::<Vec<_>>());
    for block in 0..4 {
        assert!(stack.cache.contains(&format!("{url}#{block}")).await, "block {block}");
    }
}

#[tokio::test]
async fn inv_strategy_06_an_origin_that_ignores_range_is_stored_as_one_file() {
    let stack = stack().await;
    let url = "https://o.test/clip.mp4";
    stack.fetcher.route(url, Script::ok(vec![7; 2 * MIB]).ignoring_range());
    let loader = ProxyLoader::new(stack.proxy.clone());

    let bytes = loader.load_all_blocks(url, &HashMap::new(), Priority::Prefetch, &tokio::sync::Notify::new()).await.unwrap();

    assert_eq!(bytes, (2 * MIB) as u64);
    assert!(stack.cache.contains(url).await);
}

#[tokio::test]
async fn inv_strategy_07_a_load_cut_by_a_restart_resumes_at_its_first_missing_block() {
    let first = stack().await;
    let url = "https://o.test/clip.mp4";
    let body = vec![7; 3 * MIB + MIB / 2];
    first.fetcher.route(url, Script::ok(body.clone()));
    let loader = ProxyLoader::new(first.proxy.clone());
    for block in 0..2 {
        first.proxy.fetch_block_into_store(url, block, &HashMap::new(), Priority::Prefetch).await.unwrap();
    }
    first.fetcher.route(url, Script::ok(body.clone()).cut_after(400_000));
    assert!(loader.load_all_blocks(url, &HashMap::new(), Priority::Prefetch, &tokio::sync::Notify::new()).await.is_err());
    assert!(first.cache.contains(&format!("{url}#1")).await);
    assert!(!first.cache.contains(&format!("{url}#2")).await);
    first.cache.flush().await.unwrap();
    let Stack { dir, cache, proxy, .. } = first;
    proxy.stop().await;
    drop((cache, proxy, loader));

    let second = stack_in(dir).await;
    second.fetcher.route(url, Script::ok(body));
    let loader = ProxyLoader::new(second.proxy.clone());
    let bytes = loader.load_all_blocks(url, &HashMap::new(), Priority::Prefetch, &tokio::sync::Notify::new()).await.unwrap();

    assert_eq!(bytes, (3 * MIB + MIB / 2) as u64);
    assert_eq!(ranges(&second.fetcher), vec![block_range(2, 3), block_range(3, 3)]);
    assert!(second.cache.contains(&format!("{url}#3")).await);
    let _ = &second.dir;
}

#[tokio::test]
async fn inv_strategy_01_a_whole_hls_source_lands_in_the_disk_cache_with_every_segment() {
    let stack = stack().await;
    stack.fetcher.route(MASTER, Script::ok(b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=900\nv0/index.m3u8\n".to_vec()));
    stack.fetcher.route(
        MEDIA,
        Script::ok(b"#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2.0,\ns0.m4s\n#EXTINF:2.0,\ns1.m4s\n#EXT-X-ENDLIST\n".to_vec()),
    );
    stack.fetcher.route(INIT, Script::ok(vec![1; 64]));
    stack.fetcher.route(SEGMENT_0, Script::ok(vec![2; 4000]));
    stack.fetcher.route(SEGMENT_1, Script::ok(vec![3; 4000]));
    let prefetcher = Prefetcher::new(
        Arc::new(ProxyLoader::new(stack.proxy.clone())),
        &Limits::default(),
        PrefetchLedger::new(Duration::from_secs(60), 100),
        Arc::new(SystemClock),
    );
    let mut events = prefetcher.settled();

    prefetcher.keep_whole(whole(MASTER, SourceKind::Video));

    assert_eq!(next(&mut events).await.outcome, Outcome::Loaded);
    for key in [MASTER, MEDIA, INIT, SEGMENT_0, SEGMENT_1] {
        assert!(stack.cache.contains(key).await, "{key}");
    }
}
