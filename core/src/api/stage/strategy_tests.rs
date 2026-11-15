use crate::api::source::MediaItem;
use crate::api::test_support::{eventually, eventually_async, rig, url_of, videos, Rig};
use crate::api::types::Kind;
use crate::fetch::mock::Script;
use crate::prefetch::CacheStrategy;

const MIB: usize = 1024 * 1024;

fn item(url: &str, kind: Kind, strategy: CacheStrategy) -> MediaItem {
    MediaItem { strategy, ..MediaItem::bare(url, kind) }
}

async fn stored(rig: &Rig, keys: &[String]) -> bool {
    for key in keys {
        if !rig.parts.cache.contains(key).await {
            return false;
        }
    }
    true
}

fn blocks(url: &str, count: usize) -> Vec<String> {
    (0..count).map(|block| format!("{url}#{block}")).collect()
}

fn cases() -> Vec<(MediaItem, Vec<String>)> {
    let (clip, circle, voice, photo) =
        ("https://o.test/clip.mp4", "https://o.test/circle.mp4", "https://o.test/voice.m4a", "https://o.test/photo.jpg");
    vec![
        (item(clip, Kind::Video, CacheStrategy::Whole), blocks(clip, 3)),
        (item(circle, Kind::Video, CacheStrategy::Whole), blocks(circle, 3)),
        (item(voice, Kind::Audio, CacheStrategy::Whole), blocks(voice, 3)),
        (item(photo, Kind::Image, CacheStrategy::Whole), vec![photo.to_string()]),
    ]
}

fn route_all(rig: &Rig, cases: &[(MediaItem, Vec<String>)]) {
    for (item, _) in cases {
        rig.fetcher.route(&item.url, Script::ok(vec![5; 2 * MIB + MIB / 2]));
    }
}

#[tokio::test]
async fn inv_strategy_08_a_place_that_shows_a_whole_source_downloads_it_in_full_and_hiding_it_does_not_stop_the_download() {
    let rig = rig().await;
    let cases = cases();
    route_all(&rig, &cases);

    for (item, _) in &cases {
        rig.parts.stage.visible(item.clone(), true);
        rig.parts.stage.visible(item.clone(), false);
    }

    for (item, keys) in &cases {
        eventually_async(|| stored(&rig, keys)).await;
        assert!(rig.fetcher.calls_for(&item.url) >= keys.len(), "{}", item.url);
    }
}

#[tokio::test]
async fn inv_strategy_09_a_whole_source_in_a_list_downloads_in_full_wherever_the_window_is() {
    let rig = rig().await;
    let clip = "https://o.test/far.mp4";
    rig.fetcher.route(clip, Script::ok(vec![5; 2 * MIB + MIB / 2]));
    let mut list = videos(40);
    list[35] = Some(item(clip, Kind::Video, CacheStrategy::Whole));

    rig.parts.stage.replace_window(0, list);

    let keys = blocks(clip, 3);
    eventually_async(|| stored(&rig, &keys)).await;
}

#[tokio::test]
async fn inv_strategy_09_a_whole_image_in_an_image_page_downloads_in_full_outside_the_image_window() {
    let rig = rig().await;
    let photo = "https://o.test/page30.jpg";
    rig.fetcher.route(photo, Script::ok(vec![5; 4000]));
    let pages: Vec<Vec<MediaItem>> = (0..40)
        .map(|page| vec![item(&format!("https://o.test/page{page}.jpg"), Kind::Image, if page == 30 { CacheStrategy::Whole } else { CacheStrategy::Window })])
        .collect();

    rig.parts.stage.replace_image_window("feed", 0, pages);

    let keys = [photo.to_string()];
    eventually_async(|| stored(&rig, &keys)).await;
}

#[tokio::test]
async fn inv_strategy_10_the_prefetch_window_skips_on_demand_and_whole_items() {
    for strategy in [CacheStrategy::OnDemand, CacheStrategy::Whole] {
        let rig = rig().await;
        let list: Vec<Option<MediaItem>> = (0..10)
            .map(|index| Some(item(&format!("https://o.test/page{index}.jpg"), Kind::Image, strategy)))
            .collect();
        let pages: Vec<Vec<MediaItem>> = list.iter().flatten().map(|item| vec![item.clone()]).collect();

        for index in 0..10 {
            rig.fetcher.route(&format!("https://o.test/page{index}.jpg"), Script::ok(vec![5; 100]));
        }
        rig.parts.stage.replace_image_window("feed", 0, pages);
        rig.parts.stage.move_image_window("feed", 3);
        tokio::time::timeout(std::time::Duration::from_secs(20), rig.parts.prefetcher.wait_live_at_most(0)).await.unwrap();

        let expected = if strategy == CacheStrategy::Whole { 10 } else { 0 };
        assert_eq!(rig.fetcher.calls(), expected, "{strategy:?}");
    }
}

#[tokio::test]
async fn inv_strategy_11_a_local_whole_source_is_never_fetched() {
    let rig = rig().await;
    let local = item("file:///bundle/intro.mp4", Kind::Video, CacheStrategy::Whole);

    rig.parts.stage.visible(local.clone(), true);
    rig.parts.stage.replace_window(0, vec![Some(local)]);
    eventually(|| rig.parts.stage.commands_idle()).await;
    tokio::time::timeout(std::time::Duration::from_secs(20), rig.parts.prefetcher.wait_live_at_most(0)).await.unwrap();

    assert_eq!(rig.fetcher.calls(), 0);
}

#[tokio::test]
async fn inv_strategy_09_a_default_list_still_reaches_only_the_window() {
    let rig = rig().await;
    for index in 0..40 {
        rig.fetcher.route(&url_of(index), Script::ok(vec![5; 100]));
    }

    rig.parts.stage.replace_window(0, videos(40));
    tokio::time::timeout(std::time::Duration::from_secs(20), rig.parts.prefetcher.wait_live_at_most(0)).await.unwrap();

    assert!(rig.fetcher.calls() < 10, "{}", rig.fetcher.calls());
}

#[tokio::test]
async fn inv_strategy_18_closing_the_stage_cancels_the_declared_loads() {
    let rig = rig().await;
    let photo = item("https://o.test/hung.jpg", Kind::Image, CacheStrategy::Whole);
    rig.fetcher.route(&photo.url, Script::ok(vec![5; 100]).hanging());
    rig.parts.stage.hold_wholes(1, std::iter::once(&photo));
    eventually(|| rig.fetcher.calls_for(&photo.url) == 1).await;

    rig.parts.stage.close().await;

    assert_eq!(rig.parts.prefetcher.counters().cancelled, 1);
}
