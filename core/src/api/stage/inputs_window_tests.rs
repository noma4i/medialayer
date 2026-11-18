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
        if background {
            PoolControl::foreground(stage);
        }
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
        let check = {
            let mut state = stage.state();
            let declaration = state.declarations.get_mut(&source.url).unwrap();
            let check = declaration.check.take();
            declaration.observed = check.as_ref().map(|check| check.abort_handle());
            check
        };
        if release_all {
            stage.release_all();
        } else {
            stage.visible(source.clone(), false);
        }
        assert_eq!(stage.declaration_checks(), 0, "the check of a retired declaration is still running");
        if let Some(mut check) = check {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(20), check.join()).await.unwrap();
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
async fn inv_prefetch_24_a_source_evicted_from_the_cache_is_prefetched_again_when_it_re_enters_the_window() {
    let rig = rig().await;
    for page in 0..8 {
        rig.fetcher.route(&url_of(page), Script::ok(vec![1; 2 * 1024 * 1024]));
    }
    let block = format!("{}#0", url_of(2));
    rig.parts.stage.replace_window(0, videos(8));
    eventually_async(|| async { rig.parts.cache.contains(&block).await }).await;
    let before = rig.fetcher.calls_for(&url_of(2));
    let defaults = crate::limits::Limits::default();
    rig.parts.cache.set_limits(crate::limits::Limits { budget: 1, min_budget: 1, ..defaults }).await.unwrap();
    assert!(!rig.parts.cache.contains(&block).await);
    rig.parts.cache.set_limits(defaults).await.unwrap();

    rig.parts.stage.replace_window(0, Vec::new());
    rig.parts.stage.replace_window(0, videos(8));

    eventually(|| rig.fetcher.calls_for(&url_of(2)) > before).await;
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
    assert_eq!(rig.parts.prefetcher.live_tasks(), 0);
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

    stage.replace_image_window("grid", 0, (0..4).map(page).collect::<Vec<_>>());

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
