use crate::session::WatchdogConfig;
use crate::api::slots::SlotCall;
use crate::api::source::MediaItem;
use crate::api::test_support::{eventually, eventually_async, rig, rig_with, url_of, video, videos};
use crate::api::types::{Kind, MediaState};
use crate::fetch::mock::Script;
use crate::identity::cache_key;

#[tokio::test]
async fn inv_pool_01_old_pressure_commands_preserve_new_assignments() {
    use crate::api::ports::{PoolControl, SlotDirectives};
    for background in [false, true] {
        let rig = rig().await;
        let stage = &rig.parts.stage;
        stage.replace_window(1, videos(20));
        let directives = if background {
            stage.background()
        } else {
            stage.release_under(0, crate::pool::Pressure::Critical)
        };
        stage.replace_window(10, videos(20));
        let assigned = stage.state().slot_keys.clone();
        assert!(!assigned.is_empty());
        stage.apply(directives);
        assert_eq!(stage.state().slot_keys, assigned, "stale unload removed new keys");
    }
}

#[tokio::test]
async fn inv_api_03_retired_declarations_ignore_late_cache_results() {
    for release_all in [false, true] {
        let rig = rig().await;
        let stage = &rig.parts.stage;
        let source = MediaItem::bare("file:///poster.jpg", Kind::Image);
        stage.visible(source.clone(), true);
        if release_all {
            stage.release_all();
        } else {
            stage.visible(source.clone(), false);
        }
        assert_eq!(stage.declaration_checks(), 0, "the check of a retired declaration is still running");
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert!(!rig.parts.places.is_declared(&source.url), "late cache check redeclared hidden image");
    }
}

#[tokio::test]
async fn inv_api_06_reload_restores_attached_view_and_playback_preferences() {
    let rig = rig_with(|config| config.silent = true).await;
    let stage = &rig.parts.stage;
    let key = url_of(0);
    stage.attach(&key, 42);
    stage.set_rate(&key, 1.5);
    stage.replace_window(0, videos(1));
    stage.seek(&key, 100);
    let slot = stage.state().loaded[&key].slot;
    eventually(|| rig.factory.slot_holding(&key).is_some()).await;
    let player = rig.factory.slot(slot as u32);
    player.seeked.notified().await;
    player.calls.lock().unwrap().clear();
    stage.reload_key(&key);
    stage.seek(&key, 200);
    player.seeked.notified().await;
    let calls = player.calls();
    assert!(calls.contains(&SlotCall::Attach { view_token: 42 }), "{calls:?}");
    assert!(calls.contains(&SlotCall::Mute), "{calls:?}");
    assert!(calls.contains(&SlotCall::Rate { rate: 1.5 }), "{calls:?}");
}

fn total_calls(rig: &crate::api::test_support::Rig) -> usize {
    rig.factory.slots.lock().unwrap().values().map(|slot| slot.calls().len()).sum()
}

#[tokio::test]
async fn inv_api_04_the_window_loads_the_neighbours_through_the_proxy_and_the_active_page_plays() {
    let rig = rig().await;
    let stage = &rig.parts.stage;

    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &[]);

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
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &[]);
    eventually(|| rig.factory.slot_holding(&url_of(6)).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Preroll))).await;
    eventually(|| stage.commands_idle()).await;
    let settled = total_calls(&rig);

    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &[]);
    eventually(|| stage.commands_idle()).await;

    assert_eq!(total_calls(&rig), settled);
}

#[tokio::test]
async fn inv_api_04_a_page_that_still_wants_to_play_gets_a_player_again_after_its_slot_was_taken() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &[]);
    eventually(|| rig.factory.slot_holding(&url_of(5)).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))).await;

    stage.replace_window(0, Vec::new());

    eventually(|| {
        rig.factory.slot_holding(&url_of(5)).is_some_and(|(_, slot)| {
            let calls = slot.calls();
            calls.contains(&SlotCall::Unload) && calls.last() == Some(&SlotCall::Play)
        })
    })
    .await;
}

#[tokio::test]
async fn inv_pool_01_hundreds_of_window_shifts_never_create_more_players_than_slots() {
    let rig = rig().await;
    let stage = &rig.parts.stage;

    for step in 0..300usize {
        let active = (step * 7) % 20;
        stage.replace_window(active, videos(20));
        stage.set_playing(&url_of(active), true, &[]);
    }

    eventually(|| rig.factory.slots.lock().unwrap().len() == stage.slot_count() && stage.commands_idle()).await;
    assert_eq!(rig.factory.created.lock().unwrap().len(), stage.slot_count());
}

#[tokio::test]
async fn inv_pool_02_the_latest_playing_intent_wins_and_only_one_slot_plays() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));

    stage.set_playing(&url_of(5), true, &[]);
    stage.set_playing(&url_of(6), true, &[]);

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
    stage.replace_window(5, videos(20));
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
    stage.replace_window(0, videos(20));

    stage.pin(intro);
    eventually(|| rig.factory.slot_holding(intro).is_some()).await;
    for active in [4, 9, 14] {
        stage.replace_window(active, videos(20));
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
    stage.replace_window(0, videos(20));
    stage.visible(video("https://o.test/chat1.mp4"), true);
    stage.visible(video("https://o.test/chat2.mp4"), true);

    eventually(|| rig.factory.slot_holding(intro).is_none()).await;
}

#[tokio::test]
async fn inv_api_04_a_visible_place_outside_the_window_takes_a_slot_and_gives_it_back_when_it_leaves() {
    let rig = rig_with(|config| config.defaults.slots = 4).await;
    let stage = &rig.parts.stage;
    let chat = "https://o.test/chat.mp4";
    stage.replace_window(0, videos(20));

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

    rig.parts.stage.replace_window(0, videos(8));

    eventually_async(|| async { rig.parts.cache.contains(&format!("{}#0", url_of(2))).await }).await;
    assert_eq!(rig.fetcher.calls_for(&url_of(7)), 0);
}

#[tokio::test]
async fn inv_prefetch_21_the_window_prefetches_only_after_the_playing_item_has_its_first_frame() {
    let rig = rig().await;
    for page in 0..8 {
        rig.fetcher.route(&url_of(page), Script::ok(vec![1; 2 * 1024 * 1024]));
    }
    let stage = &rig.parts.stage;
    stage.visible(video(&url_of(0)), true);
    stage.set_playing(&url_of(0), true, &[]);
    eventually(|| rig.parts.places.state(&url_of(0)) == MediaState::Loading).await;

    stage.replace_window(0, videos(8));
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!((1..4).map(|page| rig.fetcher.calls_for(&url_of(page))).sum::<usize>(), 0);

    eventually(|| rig.factory.slot_holding(&url_of(0)).is_some()).await;
    stage.ready(rig.holder(&url_of(0)).0 as usize, stage.generation_at(rig.holder(&url_of(0)).0 as usize));

    eventually_async(|| async { rig.parts.cache.contains(&format!("{}#0", url_of(2))).await }).await;
    assert_eq!(rig.fetcher.calls_for(&url_of(7)), 0);
}

#[tokio::test]
async fn inv_api_04_a_view_attaches_to_the_slot_that_holds_its_key_even_when_it_was_declared_before_the_load() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.attach(&url_of(1), 9);

    stage.replace_window(0, videos(3));

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
    stage.replace_window(5, videos(20));
    eventually(|| [4, 5, 6].iter().all(|page| rig.factory.slot_holding(&url_of(*page)).is_some())).await;

    stage.set_muted_all(true);
    stage.set_muted(&url_of(5), true);
    stage.set_muted(&url_of(5), false);
    stage.set_muted_all(false);
    stage.replace_window(9, videos(20));

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
    stage.replace_window(5, videos(20));
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
    stage.replace_window(5, videos(20));
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

    rig.parts.stage.replace_window(0, window);
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
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &[]);
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

    stage.replace_image_window("grid", 0, (0..4).map(page).collect());

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

#[tokio::test]
async fn inv_api_18_a_place_that_leaves_while_not_playing_resumes_at_its_position_when_it_returns() {
    let rig = rig_with(|config| config.defaults.slots = 4).await;
    let stage = &rig.parts.stage;
    let chat = "https://o.test/chat.mp4";
    stage.replace_window(0, videos(20));
    stage.visible(video(chat), true);
    eventually(|| rig.factory.slot_holding(chat).is_some()).await;
    rig.holder(chat).1.sample.lock().unwrap().position_ms = 14_000;
    stage.seek(chat, 14_000);
    eventually(|| stage.commands_idle()).await;
    stage.visible(video(chat), false);
    eventually(|| rig.factory.slot_holding(chat).is_none()).await;

    stage.visible(video(chat), true);

    eventually(|| rig.factory.slot_holding(chat).is_some() && stage.commands_idle()).await;
    let calls = rig.holder(chat).1.calls();
    let load = calls.iter().rposition(|call| matches!(call, SlotCall::Load { .. })).unwrap();
    let seek = calls.iter().rposition(|call| matches!(call, SlotCall::Seek { position_ms } if *position_ms >= 14_000));
    assert!(seek.is_some_and(|seek| seek > load), "{calls:?}");
}

#[tokio::test]
async fn inv_api_19_scrolling_a_feed_keeps_the_active_page_playing_and_its_neighbours_loaded() {
    let rig = rig_with(|config| config.pool.reach = 1).await;
    let stage = &rig.parts.stage;
    let all = videos(20);
    let mut shown: Vec<usize> = Vec::new();

    for active in 0..12usize {
        let rendered: Vec<usize> = (active.saturating_sub(2)..(active + 3).min(20)).collect();
        for page in shown.iter().filter(|page| !rendered.contains(page)) {
            stage.visible(all[*page].clone().unwrap(), false);
            stage.set_playing(&url_of(*page), false, &[]);
        }
        for page in rendered.iter().filter(|page| !shown.contains(page)) {
            stage.visible(all[*page].clone().unwrap(), true);
        }
        shown = rendered;
        if active > 0 {
            stage.set_playing(&url_of(active - 1), false, &[]);
        }
        stage.replace_window(active, all.clone());
        stage.set_playing(&url_of(active), true, &[]);

        eventually(|| stage.commands_idle()).await;
        let held = |page: usize| rig.factory.slot_holding(&url_of(page));
        assert!(held(active).is_some_and(|(_, slot)| slot.calls().last() == Some(&SlotCall::Play)), "active {active} is not playing");
        for page in [active.wrapping_sub(1), active + 1] {
            if page < 20 {
                assert!(held(page).is_some(), "page {page} next to active {active} holds no player");
            }
        }
    }
}

#[tokio::test]
async fn inv_api_20_declaring_a_window_costs_the_same_for_a_short_and_a_long_list() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let measure = |count: usize| {
        let sources = videos(count);
        crate::alloc_probe::allocations_of(|| stage.replace_window(count / 2, sources))
    };
    measure(20);
    measure(2000);
    let short = measure(20);
    let long = measure(2000);

    assert!(long <= short * 2, "replace_window allocated {long} times for 2000 sources and {short} for 20");
}

#[tokio::test]
async fn inv_api_21_the_mute_and_rate_of_a_place_that_left_are_forgotten() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    for page in 0..50usize {
        let source = video(&url_of(page));
        stage.visible(source.clone(), true);
        stage.set_muted(&url_of(page), true);
        stage.set_rate(&url_of(page), 1.5);
        stage.visible(source, false);
    }
    eventually(|| stage.commands_idle()).await;

    assert_eq!(stage.kept_preferences(), 0);
}

#[tokio::test]
async fn inv_api_21_a_declared_place_keeps_its_mute_when_its_player_is_unloaded_and_loaded_again() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let source = video(&url_of(5));
    stage.visible(source, true);
    stage.set_muted(&url_of(5), true);
    stage.replace_window(5, videos(20));
    eventually(|| rig.factory.slot_holding(&url_of(5)).is_some()).await;

    stage.replace_window(15, videos(20));
    eventually(|| rig.factory.slot_holding(&url_of(5)).is_none() && stage.commands_idle()).await;
    stage.replace_window(5, videos(20));
    eventually(|| rig.factory.slot_holding(&url_of(5)).is_some() && stage.commands_idle()).await;

    let (_, slot) = rig.holder(&url_of(5));
    let last = slot.calls().into_iter().rev().find(|call| matches!(call, SlotCall::Mute | SlotCall::Unmute));
    assert_eq!(last, Some(SlotCall::Mute));
}

#[tokio::test]
async fn inv_api_30_a_peeked_neighbour_plays_muted_beside_the_active_page_and_is_not_restarted_when_promoted() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &[]);
    eventually(|| rig.factory.slot_holding(&url_of(6)).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Preroll))).await;

    stage.set_peeking(&url_of(6), true);
    eventually(|| rig.holder(&url_of(6)).1.calls().contains(&SlotCall::Play)).await;
    let calls = rig.holder(&url_of(6)).1.calls();
    let mute = calls.iter().position(|call| *call == SlotCall::Mute).unwrap();
    assert!(mute < calls.iter().position(|call| *call == SlotCall::Play).unwrap());
    assert!(!rig.holder(&url_of(5)).1.calls().contains(&SlotCall::Pause));

    stage.set_playing(&url_of(6), true, &[]);
    eventually(|| rig.holder(&url_of(5)).1.calls().contains(&SlotCall::Pause) && rig.holder(&url_of(6)).1.calls().contains(&SlotCall::Unmute)).await;
    assert_eq!(rig.holder(&url_of(6)).1.calls().iter().filter(|call| **call == SlotCall::Play).count(), 1);
}

#[tokio::test]
async fn inv_api_31_a_playing_place_resumes_at_its_position_when_the_whole_window_leaves_and_returns() {
    let rig = rig_with(|config| config.defaults.slots = 4).await;
    let stage = &rig.parts.stage;
    let clip = url_of(0);
    stage.replace_window(0, videos(20));
    stage.set_playing(&clip, true, &[]);
    eventually(|| rig.factory.slot_holding(&clip).is_some()).await;
    let (_, player) = rig.holder(&clip);
    player.sample.lock().unwrap().position_ms = 5_600;
    stage.seek(&clip, 5_600);
    eventually(|| stage.commands_idle()).await;

    stage.replace_window(0, Vec::new());
    eventually(|| player.calls().contains(&SlotCall::Unload)).await;
    stage.replace_window(0, videos(20));
    stage.set_playing(&clip, true, &[]);

    eventually(|| rig.factory.slot_holding(&clip).is_some() && stage.commands_idle()).await;
    let calls = rig.holder(&clip).1.calls();
    let load = calls.iter().rposition(|call| matches!(call, SlotCall::Load { .. })).unwrap();
    let seek = calls.iter().rposition(|call| matches!(call, SlotCall::Seek { position_ms } if *position_ms >= 5_600));
    assert!(seek.is_some_and(|seek| seek > load), "{calls:?}");
}

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
    stage.replace_image_window("grid", 0, (0..30).map(|index| vec![MediaItem::bare(&remote(index), Kind::Image)]).collect());
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

fn transport(rig: &crate::api::test_support::Rig, key: &str) -> Option<SlotCall> {
    rig.factory.slot_holding(key)?.1.calls().into_iter().rfind(|call| matches!(call, SlotCall::Play | SlotCall::Pause))
}

#[tokio::test]
async fn inv_session_12_players_stay_paused_until_the_last_suspension_is_released() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &[]);
    eventually(|| rig.factory.slot_holding(&url_of(5)).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))).await;

    stage.suspend(1, &[]);
    stage.suspend(2, &[]);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Pause)).await;

    stage.resume(1);
    eventually(|| stage.commands_idle()).await;
    assert_eq!(transport(&rig, &url_of(5)), Some(SlotCall::Pause), "the first release resumed the player");

    stage.resume(2);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Play)).await;
}

#[tokio::test]
async fn inv_session_12_only_a_group_every_suspension_allows_keeps_playing() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &[]);
    stage.set_playing(&url_of(6), true, &[Some("chat".to_string())]);
    eventually(|| transport(&rig, &url_of(6)) == Some(SlotCall::Play)).await;

    stage.suspend(1, &["chat".to_string(), "voice".to_string()]);
    stage.suspend(2, &["chat".to_string()]);
    eventually(|| stage.commands_idle()).await;
    assert_eq!(transport(&rig, &url_of(6)), Some(SlotCall::Play), "a group allowed by every suspension stopped");

    stage.suspend(3, &["voice".to_string()]);
    eventually(|| transport(&rig, &url_of(6)) == Some(SlotCall::Pause)).await;
    stage.resume(3);
    eventually(|| transport(&rig, &url_of(6)) == Some(SlotCall::Play)).await;

    stage.set_playing(&url_of(6), true, &[]);
    eventually(|| transport(&rig, &url_of(6)) == Some(SlotCall::Pause)).await;
}

#[tokio::test]
async fn inv_session_14_a_key_with_several_groups_plays_if_one_group_is_allowed_by_every_suspension() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &[Some("ring".to_string()), Some("system".to_string())]);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Play)).await;

    stage.suspend(1, &["system".to_string()]);
    eventually(|| stage.commands_idle()).await;
    assert_eq!(transport(&rig, &url_of(5)), Some(SlotCall::Play), "a group allowed by the only suspension stopped");

    stage.suspend(2, &["ring".to_string()]);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Pause)).await;
    stage.resume(2);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Play)).await;
}

fn ungrouped_and_system() -> [Option<String>; 2] {
    [None, Some("system".to_string())]
}

#[tokio::test]
async fn inv_session_17_an_ungrouped_owner_counts_as_the_default_group_under_a_suspension() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &ungrouped_and_system());
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Play)).await;

    stage.suspend(1, &["ring".to_string()]);
    eventually(|| stage.commands_idle()).await;
    assert_eq!(transport(&rig, &url_of(5)), Some(SlotCall::Play), "an ungrouped owner is in the default group and was paused");
}

#[tokio::test]
async fn inv_session_17_suspensions_for_the_default_group_and_system_pause_ungrouped_and_system_owners() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &ungrouped_and_system());
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Play)).await;

    stage.suspend(1, &["ring".to_string()]);
    stage.suspend(2, &["system".to_string()]);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Pause)).await;
}

#[tokio::test]
async fn inv_session_17_the_default_group_leaves_the_set_when_the_ungrouped_owner_leaves() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &ungrouped_and_system());
    stage.suspend(1, &["ring".to_string()]);
    eventually(|| stage.commands_idle()).await;
    assert_eq!(transport(&rig, &url_of(5)), Some(SlotCall::Play));

    stage.set_playing(&url_of(5), true, &[Some("system".to_string())]);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Pause)).await;
}

#[tokio::test]
async fn inv_api_45_a_play_intent_given_before_the_source_materializes_reaches_the_slot_in_both_orders() {
    for kind in [Kind::Video, Kind::Audio] {
        for intent_first in [true, false] {
            let rig = rig().await;
            let (stage, url) = (&rig.parts.stage, url_of(0));
            let source = MediaItem::bare(&url, kind);
            if intent_first {
                stage.set_playing(&url, true, &[]);
                stage.visible(source, true);
            } else {
                stage.visible(source, true);
                stage.set_playing(&url, true, &[]);
            }
            eventually(|| rig.factory.slot_holding(&url).is_some() && stage.commands_idle()).await;
            let calls = rig.holder(&url).1.calls();

            assert_eq!(calls.iter().filter(|call| **call == SlotCall::Play).count(), 1, "{kind:?} intent_first={intent_first}: {calls:?}");
        }
    }
}

#[tokio::test]
async fn inv_api_45_only_the_elected_intent_plays_when_the_sources_materialize_after_it() {
    let rig = rig().await;
    let (stage, first, second) = (&rig.parts.stage, url_of(0), url_of(1));
    stage.set_playing(&first, true, &[]);
    stage.set_playing(&second, true, &[]);
    stage.visible(video(&first), true);
    stage.visible(video(&second), true);
    eventually(|| rig.factory.slot_holding(&first).is_some() && rig.factory.slot_holding(&second).is_some() && stage.commands_idle()).await;

    let plays = |url: &str| rig.holder(url).1.calls().iter().filter(|call| **call == SlotCall::Play).count();
    let elected: Vec<usize> = [&first, &second].iter().map(|url| usize::from(stage.state().is_playing(url))).collect();
    assert_eq!(elected.iter().sum::<usize>(), 1, "exactly one intent is elected");
    assert_eq!([plays(&first), plays(&second)], [elected[0], elected[1]], "a suppressed intent played or the elected one did not");
}
