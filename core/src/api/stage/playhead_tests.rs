use crate::api::ports::PoolControl;
use crate::api::slots::SlotCall;
use crate::memory::Step;
use crate::api::source::MediaItem;
use crate::api::test_support::{eventually, rig, MockSlot, Rig};
use crate::api::types::Kind;
use crate::fetch::mock::Script;
use crate::proxy::test_client::get;
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;

const MEDIA: &str = "https://o.test/clip/media.m3u8";
const SEGMENTS: usize = 8;

fn segment(index: usize) -> String {
    format!("https://cdn.test/clip/s{index}.ts")
}

async fn playing_clip(rig: &Rig) -> (usize, Arc<MockSlot>) {
    let mut playlist = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:4\n");
    for index in 0..SEGMENTS {
        playlist.push_str(&format!("#EXTINF:3.000,\n{}\n", segment(index)));
        rig.fetcher.route(&segment(index), Script::ok(vec![index as u8; 600]));
    }
    playlist.push_str("#EXT-X-ENDLIST\n");
    rig.fetcher.route(MEDIA, Script::ok(playlist.into_bytes()));
    rig.parts.stage.replace_window(0, vec![Some(MediaItem::bare(MEDIA, Kind::Video))]);
    rig.parts.stage.set_playing(MEDIA, true);
    eventually(|| rig.factory.slot_holding(MEDIA).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))).await;
    let (index, slot) = rig.holder(MEDIA);
    let route = slot.calls().into_iter().find_map(|call| if let SlotCall::Load { route, .. } = call { Some(route.url) } else { None }).unwrap();
    let playlist_url = Url::parse(&route).unwrap();
    let reply = get(rig.parts.proxy.port(), &format!("{}?{}", playlist_url.path(), playlist_url.query().unwrap())).await;
    assert_eq!(reply.status, 200);
    (index as usize, slot)
}

fn segment_path(index: usize) -> String {
    format!("/{}", segment(index))
}

fn readaheads(slot: &MockSlot) -> Vec<u64> {
    slot.calls().into_iter().filter_map(|call| if let SlotCall::Readahead { ms } = call { Some(ms) } else { None }).collect()
}

#[tokio::test]
async fn inv_readahead_06_the_slot_gets_the_core_hint_when_the_playlist_is_served_and_a_bigger_one_after_a_stall() {
    let rig = rig().await;
    let (slot_index, slot) = playing_clip(&rig).await;
    eventually(|| readaheads(&slot) == vec![8_000]).await;

    rig.parts.stage.stalled(slot_index, 1_000);

    eventually(|| readaheads(&slot) == vec![8_000, 12_000]).await;
}

#[tokio::test]
async fn inv_readahead_02_the_playhead_comes_from_the_loaded_slot_on_demand_and_a_seek_counts_before_the_slot_reports_it() {
    let rig = rig().await;
    let (_, slot) = playing_clip(&rig).await;
    let port = rig.parts.proxy.port();
    let held = tokio::spawn(async move { get(port, &segment_path(6)).await });
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(!held.is_finished());
    slot.sample.lock().unwrap().position_ms = 12_000;
    let started = Instant::now();
    assert_eq!(tokio::time::timeout(Duration::from_millis(1_400), held).await.expect("released by the sampled position").unwrap().status, 200);
    assert!(started.elapsed() < Duration::from_millis(1_400));

    slot.sample.lock().unwrap().position_ms = 0;
    let seek_held = tokio::spawn(async move { get(port, &segment_path(7)).await });
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(!seek_held.is_finished());
    rig.parts.stage.seek(MEDIA, 21_000);
    assert_eq!(tokio::time::timeout(Duration::from_millis(400), seek_held).await.expect("released by the seek target").unwrap().status, 200);
}

#[tokio::test]
async fn inv_readahead_07_the_memory_step_of_the_pool_reaches_the_cap_and_the_hint_of_the_slot() {
    let rig = rig().await;
    let (_, slot) = playing_clip(&rig).await;
    eventually(|| readaheads(&slot) == vec![8_000]).await;

    PoolControl::set_step(&rig.parts.stage, Step::PlayingSlotOnly);

    eventually(|| readaheads(&slot) == vec![8_000, 4_000]).await;
}

fn named(name: &str) -> String {
    format!("https://o.test/{name}/media.m3u8")
}

async fn serve_clip(rig: &Rig, name: &str) -> Arc<MockSlot> {
    let mut playlist = String::from("#EXTM3U\n#EXT-X-TARGETDURATION:4\n");
    for index in 0..SEGMENTS {
        playlist.push_str(&format!("#EXTINF:3.000,\nhttps://cdn.test/{name}/s{index}.ts\n"));
    }
    playlist.push_str("#EXT-X-ENDLIST\n");
    rig.fetcher.route(&named(name), Script::ok(playlist.into_bytes()));
    eventually(|| rig.factory.slot_holding(&named(name)).is_some()).await;
    let (_, slot) = rig.holder(&named(name));
    let route = slot.calls().into_iter().find_map(|call| if let SlotCall::Load { route, .. } = call { Some(route.url) } else { None }).unwrap();
    let url = Url::parse(&route).unwrap();
    assert_eq!(get(rig.parts.proxy.port(), &format!("{}?{}", url.path(), url.query().unwrap())).await.status, 200);
    slot
}

#[tokio::test]
async fn inv_readahead_08_the_slot_of_a_neighbour_gets_one_target_and_the_playing_one_the_full_cap() {
    let rig = rig().await;
    let items = ["a", "b", "c"].map(|name| Some(MediaItem::bare(&named(name), Kind::Video)));
    rig.parts.stage.replace_window(1, items.to_vec());
    rig.parts.stage.set_playing(&named("b"), true);
    let (previous, playing) = (serve_clip(&rig, "a").await, serve_clip(&rig, "b").await);

    eventually(|| readaheads(&playing) == vec![8_000] && readaheads(&previous) == vec![4_000]).await;
}
