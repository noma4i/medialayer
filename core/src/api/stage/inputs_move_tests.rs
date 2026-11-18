use crate::session::WatchdogConfig;
use crate::api::slots::SlotCall;
use crate::api::source::MediaItem;
use crate::api::test_support::{eventually, eventually_async, rig, rig_with, url_of, videos};
use crate::api::types::{Kind, MediaState};
use crate::fetch::mock::Script;
use crate::identity::cache_key;

#[tokio::test]
async fn inv_api_22_moving_the_active_page_reuses_the_declared_list_and_loads_the_new_neighbours() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(2, videos(20));

    stage.move_window(10);
    stage.set_playing(&url_of(10), true, &[]);

    eventually(|| [9, 10, 11].iter().all(|page| rig.factory.slot_holding(&url_of(*page)).is_some())).await;
    eventually(|| rig.holder(&url_of(10)).1.calls().contains(&SlotCall::Play)).await;
    assert!(rig.factory.slot_holding(&url_of(2)).is_none());
}

#[tokio::test]
async fn inv_api_22_moving_before_any_list_was_declared_or_after_release_all_does_nothing() {
    let rig = rig().await;
    let stage = &rig.parts.stage;

    stage.move_window(3);
    stage.replace_window(0, videos(20));
    stage.release_all();
    stage.move_window(7);

    eventually(|| stage.commands_idle()).await;
    assert!(rig.factory.slot_holding(&url_of(7)).is_none());
    assert_eq!(rig.parts.prefetcher.live_tasks(), 0);
}

#[tokio::test]
async fn inv_api_22_moving_an_image_window_prefetches_the_pages_around_the_new_position_from_the_declared_pages() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let remote = |index: usize| format!("https://o.test/poster{index}.jpg");
    for index in 0..30 {
        rig.fetcher.route(&remote(index), Script::ok(vec![index as u8; 32]));
    }
    stage.replace_image_window("grid", 0, (0..30).map(|index| vec![MediaItem::bare(&remote(index), Kind::Image)]).collect::<Vec<_>>());
    eventually_async(|| async { rig.parts.cache.contains(&cache_key(&remote(0))).await }).await;
    assert_eq!(rig.fetcher.calls_for(&remote(25)), 0);

    stage.move_image_window("grid", 25);

    eventually_async(|| async { rig.parts.cache.contains(&cache_key(&remote(25))).await }).await;
    assert_eq!(rig.fetcher.calls_for(&remote(25)), 1);
}

#[tokio::test]
async fn inv_api_22_a_place_shown_by_two_views_stays_ready_when_one_view_leaves() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let url = "https://o.test/chat.gif";
    let image = || MediaItem::bare(url, Kind::Image);
    stage.visible(image(), true);
    rig.parts.places.item_state(url, MediaState::Ready);
    stage.visible(image(), true);
    stage.visible(image(), false);

    eventually(|| stage.commands_idle()).await;

    assert_eq!(rig.parts.places.state(url), MediaState::Ready, "2 views appeared, 1 left, 1 still shows the place");
    stage.visible(image(), false);
    assert_eq!(rig.parts.places.state(url), MediaState::Idle, "the last view left");
}

#[tokio::test]
async fn inv_api_36_a_seek_before_the_key_loads_reaches_the_slot_after_load() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let key = url_of(0);

    stage.seek(&key, 700);
    stage.replace_window(0, videos(1));

    eventually(|| rig.factory.slot_holding(&key).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Seek { position_ms: 700 }))).await;
    let calls = rig.holder(&key).1.calls();
    let position = |wanted: &dyn Fn(&SlotCall) -> bool| calls.iter().position(wanted).unwrap();
    assert!(position(&|call| matches!(call, SlotCall::Load { .. })) < position(&|call| matches!(call, SlotCall::Seek { .. })), "{calls:?}");
}

#[tokio::test]
async fn inv_api_37_a_fraction_chosen_before_the_duration_is_known_becomes_a_seek_when_it_is() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let key = url_of(0);

    stage.seek_fraction(&key, 0.5);
    stage.replace_window(0, videos(1));
    eventually(|| rig.factory.slot_holding(&key).is_some()).await;
    assert!(!rig.holder(&key).1.calls().iter().any(|call| matches!(call, SlotCall::Seek { .. })));
    stage.set_playing(&key, true, &[]);

    eventually(|| rig.holder(&key).1.calls().contains(&SlotCall::Seek { position_ms: 30_000 })).await;
}

#[tokio::test]
async fn inv_api_38_a_pending_fraction_becomes_a_seek_when_the_slot_reports_ready() {
    let rig = rig_with(|config| {
        config.watchdog = WatchdogConfig { first_window_ms: 60_000, progress_window_ms: 60_000, startup_delay_ms: 60_000, ..crate::api::test_support::fast_watchdog() };
    })
    .await;
    let stage = &rig.parts.stage;
    let key = url_of(0);

    stage.seek_fraction(&key, 0.5);
    stage.replace_window(0, videos(1));
    eventually(|| rig.factory.slot_holding(&key).is_some()).await;
    let (slot, player) = rig.holder(&key);
    player.sample.lock().unwrap().duration_ms = None;
    stage.set_playing(&key, true, &[]);
    eventually(|| player.calls().contains(&SlotCall::Play)).await;
    assert!(!player.calls().iter().any(|call| matches!(call, SlotCall::Seek { .. })));

    player.sample.lock().unwrap().duration_ms = Some(60_000);
    stage.ready(slot as usize, stage.generation_at(slot as usize));

    eventually(|| player.calls().contains(&SlotCall::Seek { position_ms: 30_000 })).await;
}

#[tokio::test]
async fn inv_api_39_a_ready_without_duration_keeps_the_fraction_for_the_next_slot_event_with_one() {
    let rig = rig_with(|config| {
        config.watchdog = WatchdogConfig { first_window_ms: 60_000, progress_window_ms: 60_000, startup_delay_ms: 60_000, ..crate::api::test_support::fast_watchdog() };
    })
    .await;
    let stage = &rig.parts.stage;
    let key = url_of(0);

    stage.seek_fraction(&key, 0.6);
    stage.replace_window(0, videos(1));
    eventually(|| rig.factory.slot_holding(&key).is_some()).await;
    let (slot, player) = rig.holder(&key);
    player.sample.lock().unwrap().duration_ms = None;

    stage.ready(slot as usize, stage.generation_at(slot as usize));
    assert!(!player.calls().iter().any(|call| matches!(call, SlotCall::Seek { .. })), "no duration yet");

    player.sample.lock().unwrap().duration_ms = Some(10_000);
    stage.ready(slot as usize, stage.generation_at(slot as usize));

    eventually(|| player.calls().contains(&SlotCall::Seek { position_ms: 6_000 })).await;
}
