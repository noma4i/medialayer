use super::runner_tests::{eventually, quiet};
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
    quiet().await;
    let running = started(&stack, urls);

    prefetcher.release_whole(1, &whole(&urls[0], kind_of(&urls[0])));
    eventually(|| started(&stack, urls) > running).await;
    quiet().await;
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
