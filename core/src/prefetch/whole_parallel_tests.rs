use super::runner_tests::eventually;
use super::whole_dedupe_tests::over_proxy;
use super::whole_tests::{stack, Stack};
use super::*;
use crate::fetch::mock::Script;
use crate::limits::Limits;
use std::collections::HashMap;

fn whole(url: &str, kind: SourceKind) -> Source {
    Source { url: url.to_string(), headers: HashMap::new(), kind, strategy: CacheStrategy::Whole }
}

fn started(stack: &Stack, urls: &[String]) -> usize {
    urls.iter().filter(|url| stack.fetcher.calls_for(url) > 0).count()
}

fn kind_of(url: &str) -> SourceKind {
    match url.rsplit('.').next() {
        Some("jpg") => SourceKind::Image,
        Some("mp3") => SourceKind::Audio,
        _ => SourceKind::Video,
    }
}

async fn parallel(max_concurrent: u32, urls: &[String], expected: usize) -> (usize, usize) {
    let stack = stack().await;
    stack.proxy.set_limits(&Limits { max_concurrent, ..Limits::default() });
    for url in urls {
        stack.fetcher.route(url, Script::ok(vec![7; 2048]).hanging());
    }
    let prefetcher = over_proxy(&stack);
    for url in urls {
        prefetcher.hold_whole(1, whole(url, kind_of(url)));
    }
    eventually(|| started(&stack, urls) >= expected).await;
    let image = kind_of(&urls[0]) == SourceKind::Image;
    let whole_limit = if image { 2 } else { 1 };
    prefetcher.wait_whole_queued(image, urls.len() - whole_limit).await;
    if expected < whole_limit {
        eventually(|| stack.proxy.queued_requests() == whole_limit - expected).await;
    }
    let running = started(&stack, urls);

    prefetcher.release_whole(1, &whole(&urls[0], kind_of(&urls[0])));
    eventually(|| started(&stack, urls) > running).await;
    prefetcher.wait_whole_queued(image, urls.len() - whole_limit - 1).await;
    if expected < whole_limit {
        eventually(|| stack.proxy.queued_requests() == whole_limit - expected).await;
    }
    (running, started(&stack, urls))
}

#[tokio::test]
async fn inv_strategy_29_whole_images_load_two_at_a_time_less_the_slot_reserved_for_shown_images() {
    let urls: Vec<String> = (0..3).map(|index| format!("https://o.test/{index}.jpg")).collect();
    for (max_concurrent, expected) in [(2, 1), (3, 2), (8, 2)] {
        let (running, after_release) = parallel(max_concurrent, &urls, expected).await;
        assert_eq!((running, after_release), (expected, expected + 1), "max_concurrent {max_concurrent}");
    }
}

#[tokio::test]
async fn inv_strategy_30_whole_videos_and_audios_load_one_at_a_time_in_declaration_order() {
    let urls: Vec<String> = ["https://o.test/0.mp4", "https://o.test/1.mp3", "https://o.test/2.mp4"].map(String::from).to_vec();
    let (running, after_release) = parallel(8, &urls, 1).await;
    assert_eq!((running, after_release), (1, 2));
}

#[tokio::test]
async fn inv_strategy_34_a_shown_place_reads_through_the_proxy_while_the_whole_queue_is_busy() {
    let stack = stack().await;
    let busy = "https://o.test/busy.mp4";
    let shown = "https://o.test/shown.mp4";
    stack.fetcher.route(busy, Script::ok(vec![7; 2048]).hanging());
    stack.fetcher.route(shown, Script::ok(vec![9; 4096]));
    let prefetcher = over_proxy(&stack);
    prefetcher.keep_whole(whole(busy, SourceKind::Video));
    eventually(|| stack.fetcher.calls_for(busy) > 0).await;
    prefetcher.keep_shown_whole(whole(shown, SourceKind::Video));
    prefetcher.wait_whole_queued(false, 1).await;

    let parsed = url::Url::parse(&stack.proxy.url_for(shown, &HashMap::new()).unwrap()).unwrap();
    let target = format!("{}?{}", parsed.path(), parsed.query().unwrap_or_default());
    let reply = tokio::time::timeout(std::time::Duration::from_secs(3), crate::proxy::test_client::request(stack.proxy.port(), "GET", &target, &[("Range", "bytes=0-1023")]))
        .await
        .expect("the shown place waited for the whole queue");
    assert_eq!(reply.body.len(), 1024);
}

#[tokio::test]
async fn inv_strategy_35_a_window_image_of_a_shown_place_loads_while_whole_loads_hold_the_queues() {
    for max_concurrent in [2u32, 3, 8] {
        let stack = stack().await;
        stack.proxy.set_limits(&Limits { max_concurrent, ..Limits::default() });
        let held: Vec<String> = ["https://o.test/a.jpg", "https://o.test/b.jpg", "https://o.test/clip.mp4"].map(String::from).to_vec();
        for url in &held {
            stack.fetcher.route(url, Script::ok(vec![7; 2048]).hanging());
        }
        let poster = "https://o.test/poster.jpg";
        stack.fetcher.route(poster, Script::ok(vec![5; 2048]));
        let prefetcher = over_proxy(&stack);
        for url in &held {
            prefetcher.keep_whole(whole(url, kind_of(url)));
        }
        eventually(|| started(&stack, &held) >= 2).await;
        let loaded = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            stack.proxy.fetch_into_store(poster, &HashMap::new(), crate::proxy::Priority::Image, None),
        )
        .await;
        assert!(loaded.is_ok(), "the shown window image waited for whole loads, max_concurrent {max_concurrent}");
    }
}

#[tokio::test]
async fn inv_strategy_35_a_window_image_of_a_url_queued_as_whole_loads_without_the_queue() {
    let stack = stack().await;
    stack.proxy.set_limits(&Limits { max_concurrent: 8, ..Limits::default() });
    let held: Vec<String> = ["https://o.test/a.jpg", "https://o.test/b.jpg"].map(String::from).to_vec();
    for url in &held {
        stack.fetcher.route(url, Script::ok(vec![7; 2048]).hanging());
    }
    let poster = "https://o.test/poster.jpg";
    stack.fetcher.route(poster, Script::ok(vec![5; 2048]));
    let prefetcher = over_proxy(&stack);
    for url in &held {
        prefetcher.keep_whole(whole(url, SourceKind::Image));
    }
    eventually(|| started(&stack, &held) >= 2).await;
    prefetcher.keep_shown_whole(whole(poster, SourceKind::Image));
    prefetcher.wait_whole_queued(true, 1).await;
    let loaded = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        stack.proxy.fetch_into_store(poster, &HashMap::new(), crate::proxy::Priority::Image, None),
    )
    .await;
    assert!(loaded.is_ok(), "the shown window image waited for the whole queue of its own url");
}
