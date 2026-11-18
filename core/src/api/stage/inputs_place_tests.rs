use crate::api::slots::SlotCall;
use crate::api::source::MediaItem;
use crate::api::test_support::{eventually, rig, rig_with, url_of, video, videos};
use crate::api::types::Kind;

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
async fn inv_api_68_a_window_handed_over_shared_is_held_without_copying_the_list() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let shared = std::sync::Arc::new(videos(2000));
    let allocations = crate::alloc_probe::allocations_of(|| stage.replace_window(1000, shared.clone()));
    assert_eq!(std::sync::Arc::strong_count(&shared), 2, "the stage holds the same list, not a copy");
    let again = crate::alloc_probe::allocations_of(|| stage.replace_window(1000, shared.clone()));
    assert!(again <= allocations.max(1) * 2, "{again} vs {allocations}");
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
