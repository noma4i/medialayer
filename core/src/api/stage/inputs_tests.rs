use crate::api::slots::SlotCall;
use crate::api::source::MediaItem;
use crate::api::test_support::{eventually, eventually_async, rig, rig_with, url_of, video, videos};
use crate::api::types::Kind;
use crate::fetch::mock::Script;
use crate::identity::cache_key;

fn total_calls(rig: &crate::api::test_support::Rig) -> usize {
    rig.factory.slots.lock().unwrap().values().map(|slot| slot.calls().len()).sum()
}

#[tokio::test]
async fn inv_api_04_the_window_loads_the_neighbours_through_the_proxy_and_the_active_page_plays() {
    let rig = rig().await;
    let stage = &rig.parts.stage;

    stage.set_window(5, videos(20));
    stage.set_playing(&url_of(5), true);

    eventually(|| [4, 5, 6].iter().all(|page| rig.factory.slot_holding(&url_of(*page)).is_some())).await;
    eventually(|| rig.holder(&url_of(5)).1.calls().contains(&SlotCall::Play)).await;
    let (_, active) = rig.holder(&url_of(5));
    let route = active.calls().into_iter().find_map(|call| if let SlotCall::Load { route, .. } = call { Some(route) } else { None }).unwrap();
    assert!(route.url.starts_with(&format!("http://127.0.0.1:{}/", rig.parts.proxy.port())), "{}", route.url);
    for page in [4, 6] {
        eventually(|| rig.holder(&url_of(page)).1.calls().contains(&SlotCall::Preroll)).await;
        assert!(!rig.holder(&url_of(page)).1.calls().contains(&SlotCall::Play));
    }
}

#[tokio::test]
async fn inv_api_04_declaring_the_same_window_again_sends_no_commands() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.set_window(5, videos(20));
    stage.set_playing(&url_of(5), true);
    eventually(|| rig.factory.slot_holding(&url_of(6)).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Preroll))).await;
    eventually(|| stage.commands_idle()).await;
    let settled = total_calls(&rig);

    stage.set_window(5, videos(20));
    stage.set_playing(&url_of(5), true);
    eventually(|| stage.commands_idle()).await;

    assert_eq!(total_calls(&rig), settled);
}

#[tokio::test]
async fn inv_api_04_a_page_that_still_wants_to_play_gets_a_player_again_after_its_slot_was_taken() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.set_window(5, videos(20));
    stage.set_playing(&url_of(5), true);
    eventually(|| rig.factory.slot_holding(&url_of(5)).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))).await;

    stage.set_window(0, Vec::new());
    eventually(|| rig.factory.slot_holding(&url_of(5)).is_none() && stage.commands_idle()).await;

    eventually(|| rig.factory.slot_holding(&url_of(5)).is_some_and(|(_, slot)| slot.calls().last() == Some(&SlotCall::Play))).await;
}

#[tokio::test]
async fn inv_pool_01_hundreds_of_window_shifts_never_create_more_players_than_slots() {
    let rig = rig().await;
    let stage = &rig.parts.stage;

    for step in 0..300usize {
        let active = (step * 7) % 20;
        stage.set_window(active, videos(20));
        stage.set_playing(&url_of(active), true);
    }

    eventually(|| rig.factory.slots.lock().unwrap().len() == stage.slot_count() && stage.commands_idle()).await;
    assert_eq!(rig.factory.created.lock().unwrap().len(), stage.slot_count());
}

#[tokio::test]
async fn inv_pool_02_the_latest_playing_intent_wins_and_only_one_slot_plays() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.set_window(5, videos(20));

    stage.set_playing(&url_of(5), true);
    stage.set_playing(&url_of(6), true);

    eventually(|| [5, 6].iter().all(|page| rig.factory.slot_holding(&url_of(*page)).is_some())).await;
    eventually(|| rig.holder(&url_of(6)).1.calls().contains(&SlotCall::Play)).await;
    eventually(|| rig.holder(&url_of(5)).1.calls().last() == Some(&SlotCall::Pause)).await;
    let playing = |page| rig.holder(&url_of(page)).1.calls().iter().rev().find(|call| matches!(call, SlotCall::Play | SlotCall::Pause)).cloned();
    assert_eq!((playing(5), playing(6)), (Some(SlotCall::Pause), Some(SlotCall::Play)));
}

#[tokio::test]
async fn inv_pool_12_muting_all_or_one_key_changes_only_the_volume_and_only_when_the_value_changes() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.set_window(5, videos(20));
    eventually(|| [4, 5, 6].iter().all(|page| rig.factory.slot_holding(&url_of(*page)).is_some())).await;
    let mutes = |page| rig.holder(&url_of(page)).1.calls().iter().filter(|call| **call == SlotCall::Mute).count();

    stage.set_muted(&url_of(5), true);
    stage.set_muted(&url_of(5), true);
    eventually(|| mutes(5) == 1).await;
    assert_eq!((mutes(4), mutes(6)), (0, 0));
    stage.set_muted_all(true);
    stage.set_muted_all(true);

    eventually(|| mutes(4) == 1 && mutes(6) == 1 && stage.commands_idle()).await;
    assert_eq!((mutes(4), mutes(5), mutes(6)), (1, 1, 1));
}

#[tokio::test]
async fn inv_api_04_a_pinned_key_survives_window_moves_and_release_all_frees_every_slot() {
    let rig = rig_with(|config| config.defaults.slots = 4).await;
    let stage = &rig.parts.stage;
    let intro = "https://o.test/intro.mp4";
    stage.set_window(0, videos(20));

    stage.pin(intro);
    eventually(|| rig.factory.slot_holding(intro).is_some()).await;
    for active in [4, 9, 14] {
        stage.set_window(active, videos(20));
    }
    eventually(|| stage.commands_idle()).await;
    let (_, pinned) = rig.holder(intro);
    assert_eq!(pinned.calls().iter().filter(|call| **call == SlotCall::Unload).count(), 0);

    stage.release_all();

    eventually(|| rig.factory.slots.lock().unwrap().values().all(|slot| slot.calls().last() == Some(&SlotCall::Unload))).await;
    assert_eq!(rig.parts.prefetcher.live_tasks(), 0);
}

#[tokio::test]
async fn inv_api_04_an_unpinned_key_goes_back_to_the_normal_rules() {
    let rig = rig_with(|config| config.defaults.slots = 4).await;
    let stage = &rig.parts.stage;
    let intro = "https://o.test/intro.mp4";
    stage.pin(intro);
    eventually(|| rig.factory.slot_holding(intro).is_some()).await;

    stage.unpin(intro);
    stage.set_window(0, videos(20));
    stage.visible(video("https://o.test/chat1.mp4"), true);
    stage.visible(video("https://o.test/chat2.mp4"), true);

    eventually(|| rig.factory.slot_holding(intro).is_none()).await;
}

#[tokio::test]
async fn inv_api_04_a_visible_place_outside_the_window_takes_a_slot_and_gives_it_back_when_it_leaves() {
    let rig = rig_with(|config| config.defaults.slots = 4).await;
    let stage = &rig.parts.stage;
    let chat = "https://o.test/chat.mp4";
    stage.set_window(0, videos(20));

    stage.visible(video(chat), true);
    eventually(|| rig.factory.slot_holding(chat).is_some()).await;
    stage.visible(video(chat), false);

    eventually(|| rig.factory.slot_holding(chat).is_none()).await;
    assert!(rig.factory.slot_holding(&url_of(0)).is_some());
}

#[tokio::test]
async fn inv_api_04_the_window_also_drives_prefetch_ahead_of_the_active_page() {
    let rig = rig().await;
    for page in 0..8 {
        rig.fetcher.route(&url_of(page), Script::ok(vec![1; 2 * 1024 * 1024]));
    }

    rig.parts.stage.set_window(0, videos(8));

    eventually_async(|| async { rig.parts.cache.contains(&format!("{}#0", url_of(2))).await }).await;
    assert_eq!(rig.fetcher.calls_for(&url_of(7)), 0);
}

#[tokio::test]
async fn inv_api_04_a_view_attaches_to_the_slot_that_holds_its_key_even_when_it_was_declared_before_the_load() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.attach(&url_of(1), 9);

    stage.set_window(0, videos(3));

    eventually(|| rig.factory.slot_holding(&url_of(1)).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Attach { view_token: 9 }))).await;
    let calls = rig.holder(&url_of(1)).1.calls();
    let position = |wanted: &dyn Fn(&SlotCall) -> bool| calls.iter().position(wanted).unwrap();
    assert!(position(&|call| matches!(call, SlotCall::Load { .. })) < position(&|call| matches!(call, SlotCall::Attach { .. })));

    stage.attach(&url_of(1), 11);
    stage.seek(&url_of(1), 4_200);
    stage.detach(&url_of(1));
    let (_, player) = rig.holder(&url_of(1));
    eventually(|| player.calls().last() == Some(&SlotCall::Detach)).await;
    let calls = player.calls();
    assert!(calls.contains(&SlotCall::Attach { view_token: 11 }));
    assert!(calls.contains(&SlotCall::Seek { position_ms: 4_200 }));
}

fn sound_calls(rig: &crate::api::test_support::Rig) -> Vec<SlotCall> {
    rig.factory.slots.lock().unwrap().values().flat_map(|slot| slot.calls()).filter(|call| matches!(call, SlotCall::Mute | SlotCall::Unmute)).collect()
}

#[tokio::test]
async fn inv_api_14_a_silent_layer_never_unmutes_a_slot_whatever_the_places_ask() {
    let rig = rig_with(|config| config.silent = true).await;
    let stage = &rig.parts.stage;
    stage.set_window(5, videos(20));
    eventually(|| [4, 5, 6].iter().all(|page| rig.factory.slot_holding(&url_of(*page)).is_some())).await;

    stage.set_muted_all(true);
    stage.set_muted(&url_of(5), true);
    stage.set_muted(&url_of(5), false);
    stage.set_muted_all(false);
    stage.set_window(9, videos(20));

    eventually(|| [8, 9, 10].iter().all(|page| rig.factory.slot_holding(&url_of(*page)).is_some()) && stage.commands_idle()).await;
    assert!(!sound_calls(&rig).contains(&SlotCall::Unmute));
    for slot in rig.factory.slots.lock().unwrap().values() {
        let calls = slot.calls();
        for index in calls.iter().enumerate().filter(|(_, call)| matches!(call, SlotCall::Load { .. })).map(|(index, _)| index) {
            assert_eq!(calls.get(index + 1), Some(&SlotCall::Mute), "{calls:?}");
        }
    }
}

#[tokio::test]
async fn inv_api_14_a_layer_that_is_not_silent_still_unmutes_on_request() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.set_window(5, videos(20));
    eventually(|| rig.factory.slot_holding(&url_of(5)).is_some()).await;

    stage.set_muted(&url_of(5), true);
    stage.set_muted(&url_of(5), false);

    eventually(|| rig.holder(&url_of(5)).1.calls().contains(&SlotCall::Unmute)).await;
}

#[tokio::test]
async fn inv_api_16_a_rate_is_sent_after_the_load_of_its_key_and_reaches_the_slot_that_holds_it_within_bounds() {
    let rig = rig_with(|config| config.defaults.slots = 4).await;
    let stage = &rig.parts.stage;
    let rates = |page: usize| rig.holder(&url_of(page)).1.calls().into_iter().filter_map(|call| if let SlotCall::Rate { rate } = call { Some(rate) } else { None }).collect::<Vec<f32>>();

    stage.set_rate(&url_of(5), 1.5);
    stage.set_window(5, videos(20));
    eventually(|| rig.factory.slot_holding(&url_of(5)).is_some_and(|(_, slot)| slot.calls().iter().any(|call| matches!(call, SlotCall::Rate { .. })))).await;
    let calls = rig.holder(&url_of(5)).1.calls();
    let position = |wanted: &dyn Fn(&SlotCall) -> bool| calls.iter().position(wanted).unwrap();
    assert!(position(&|call| matches!(call, SlotCall::Load { .. })) < position(&|call| matches!(call, SlotCall::Rate { .. })));

    stage.set_rate(&url_of(5), 2.0);
    stage.set_rate(&url_of(5), f32::NAN);
    stage.set_rate(&url_of(5), 40.0);
    stage.set_rate(&url_of(5), 0.0);
    eventually(|| rates(5).len() == 4).await;
    assert_eq!(rates(5), vec![1.5, 2.0, 4.0, 0.25]);

    stage.set_rate(&url_of(5), 1.0);
    eventually(|| rates(5).len() == 5).await;
    assert_eq!(rates(5).last(), Some(&1.0));
}

#[tokio::test]
async fn inv_api_15_a_local_file_in_the_window_is_never_prefetched_and_plays_from_its_own_path() {
    let rig = rig().await;
    let local = |index: usize| MediaItem::bare_video(&format!("file:///bundle/clip{index}.mp4"));
    let window: Vec<Option<MediaItem>> = (0..4).map(|index| Some(local(index))).collect();

    rig.parts.stage.set_window(0, window);
    rig.parts.stage.visible(local(0), true);

    eventually(|| rig.factory.slot_holding("file:///bundle/clip0.mp4").is_some()).await;
    eventually(|| rig.parts.stage.commands_idle()).await;
    let (_, slot) = rig.holder("file:///bundle/clip0.mp4");
    let route = slot.calls().into_iter().find_map(|call| if let SlotCall::Load { route, .. } = call { Some(route) } else { None }).unwrap();
    assert_eq!((route.url.as_str(), route.generation), ("file:///bundle/clip0.mp4", None));
    assert_eq!(rig.parts.prefetcher.live_tasks(), 0);
    assert_eq!(rig.fetcher.calls(), 0);
    assert_eq!(rig.parts.cache.stats().await.file_count, 0);
}

#[tokio::test]
async fn inv_api_17_an_image_window_prefetches_remote_images_into_the_cache_and_never_touches_the_pool_or_local_files() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.set_window(5, videos(20));
    stage.set_playing(&url_of(5), true);
    eventually(|| rig.factory.slot_holding(&url_of(6)).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Preroll))).await;
    eventually(|| stage.commands_idle()).await;
    let settled = total_calls(&rig);
    let remote = |index: usize| format!("https://o.test/poster{index}.jpg");
    let page = |index: usize| {
        vec![MediaItem::bare(&remote(index), Kind::Image), MediaItem::bare(&format!("file:///bundle/poster{index}.jpg"), Kind::Image)]
    };
    for index in 0..4 {
        rig.fetcher.route(&remote(index), Script::ok(vec![index as u8; 32]));
    }

    stage.set_image_window("grid", 0, (0..4).map(page).collect());

    for index in 0..4 {
        eventually_async(|| async { rig.parts.cache.contains(&cache_key(&remote(index))).await }).await;
    }
    for index in 0..4 {
        assert_eq!(rig.fetcher.calls_for(&remote(index)), 1);
        assert_eq!(rig.fetcher.calls_for(&format!("file:///bundle/poster{index}.jpg")), 0);
    }
    assert_eq!(rig.parts.stage.slot_count(), rig.factory.created.lock().unwrap().len());
    eventually(|| stage.commands_idle()).await;
    assert_eq!(total_calls(&rig), settled);
}
