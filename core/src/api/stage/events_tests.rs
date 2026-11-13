use crate::api::ports::{PoolControl, SlotDirectives as _};
use crate::api::slots::SlotCall;
use crate::api::source::MediaItem;
use crate::api::test_support::{eventually, eventually_async, rig, rig_with, url_of, video, videos, Rig};
use crate::api::types::{Kind, MediaState};
use crate::identity::cache_key;
use crate::cache;
use crate::session::WatchdogConfig;
use std::sync::{Arc, Mutex};

const HLS: &str = "https://o.test/hls/master.m3u8";

fn once(url: &str) -> MediaItem {
    MediaItem { looping: false, ..video(url) }
}

fn hls() -> MediaItem {
    MediaItem::bare(HLS, Kind::Video)
}

async fn playing(rig: &Rig, source: MediaItem) -> (u32, Arc<crate::api::test_support::MockSlot>) {
    let url = source.url.clone();
    rig.parts.stage.replace_window(0, vec![Some(source)]);
    rig.parts.stage.set_playing(&url, true, &[]);
    eventually(|| rig.factory.slot_holding(&url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))).await;
    rig.holder(&url)
}

#[tokio::test]
async fn inv_api_03_the_ready_events_move_a_declared_place_to_ready() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(0, videos(3));
    stage.visible(video(&url_of(0)), true);
    eventually(|| rig.factory.slot_holding(&url_of(0)).is_some()).await;
    let (slot, _) = rig.holder(&url_of(0));

    stage.ready(slot as usize, stage.generation_at(slot as usize));
    assert_eq!(rig.parts.places.state(&url_of(0)), MediaState::Ready);
    stage.visual_ready(slot as usize, stage.generation_at(slot as usize));

    assert_eq!(rig.parts.places.state(&url_of(0)), MediaState::Rendered);
}

#[tokio::test]
async fn inv_placeholder_03_a_video_already_in_the_cache_is_declared_ready_without_a_loading_step() {
    let rig = rig().await;
    rig.parts.cache.put(&cache_key(&url_of(0)), cache::Kind::Stream, b"bytes").await.unwrap();
    let seen: Arc<Mutex<Vec<MediaState>>> = Arc::default();
    let sink = seen.clone();
    let _subscription = rig.parts.places.subscribe(&url_of(0), Arc::new(move |state| sink.lock().unwrap().push(state)));

    rig.parts.stage.visible(video(&url_of(0)), true);

    eventually(|| seen.lock().unwrap().last() == Some(&MediaState::Ready)).await;
    assert!(!seen.lock().unwrap().contains(&MediaState::Loading));
}

#[tokio::test]
async fn inv_api_06_a_startup_http_error_on_a_proxied_playlist_refreshes_the_route_and_loads_it_again() {
    let rig = rig().await;
    let (slot, player) = playing(&rig, hls()).await;
    assert_eq!(player.loads(), 1);

    rig.parts.stage.failed(slot as usize, rig.parts.stage.generation_at(slot as usize), 404, String::new());

    eventually(|| player.loads() == 2).await;
    assert_eq!(player.calls().iter().filter(|call| **call == SlotCall::Unload).count(), 1);
}

#[tokio::test]
async fn inv_api_06_repeated_playback_failures_reload_the_player_and_then_fail_the_place_with_the_reason() {
    let rig = rig().await;
    rig.parts.places.declared(&url_of(0), false);
    let (slot, player) = playing(&rig, video(&url_of(0))).await;

    for _ in 0..6 {
        rig.parts.stage.failed(slot as usize, rig.parts.stage.generation_at(slot as usize), 0, "decoder died".to_string());
    }

    assert_eq!(rig.parts.places.state(&url_of(0)), MediaState::Failed { detail: "decoder died".to_string() });
    eventually(|| player.loads() >= 2).await;
}

#[tokio::test]
async fn inv_api_06_a_silent_player_is_reloaded_by_the_watchdog() {
    let rig = rig().await;
    let (slot, silent) = playing(&rig, video(&url_of(0))).await;
    rig.parts.stage.ready(slot as usize, rig.parts.stage.generation_at(slot as usize));

    eventually(|| silent.loads() >= 2).await;
}

#[tokio::test]
async fn inv_api_06_a_player_whose_clock_moves_is_checked_again_and_left_alone() {
    let rig = rig_with(|_| {}).await;
    *rig.factory.advance_ms.lock().unwrap() = 1_000;
    let (slot, healthy) = playing(&rig, video(&url_of(0))).await;
    rig.parts.stage.ready(slot as usize, rig.parts.stage.generation_at(slot as usize));

    eventually(|| healthy.samples.load(std::sync::atomic::Ordering::SeqCst) >= 3).await;

    assert_eq!(healthy.loads(), 1);
}

#[tokio::test]
async fn inv_api_06_a_stall_arms_the_watchdog_only_for_the_playing_slot() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(0, videos(3));
    eventually(|| rig.factory.slot_holding(&url_of(1)).is_some()).await;
    let (idle_slot, idle) = rig.holder(&url_of(1));
    stage.stalled(idle_slot as usize, stage.generation_at(idle_slot as usize), 0);
    assert_eq!(stage.armed_tasks(), 0);
    assert_eq!(idle.samples.load(std::sync::atomic::Ordering::SeqCst), 0);

    stage.set_playing(&url_of(0), true, &[]);
    eventually(|| rig.factory.slot_holding(&url_of(0)).is_some()).await;
    let (active_slot, active) = rig.holder(&url_of(0));
    stage.stalled(active_slot as usize, stage.generation_at(active_slot as usize), 0);
    assert_eq!(stage.armed_tasks(), 1);

    eventually(|| active.samples.load(std::sync::atomic::Ordering::SeqCst) >= 1).await;
}

#[tokio::test]
async fn inv_api_06_going_to_the_background_unloads_playback_and_returning_resumes_it() {
    let rig = rig().await;
    let (_, player) = playing(&rig, video(&url_of(0))).await;
    let stage = &rig.parts.stage;

    let directives = PoolControl::background(stage);
    stage.apply(directives);
    eventually(|| player.calls().last() == Some(&SlotCall::Unload)).await;
    PoolControl::foreground(stage);

    eventually(|| player.calls().iter().filter(|call| **call == SlotCall::Play).count() >= 2).await;
}

#[tokio::test]
async fn inv_api_06_watchdog_escalation_purges_the_cached_source_and_loads_it_again() {
    let rig = rig_with(|config| config.watchdog = WatchdogConfig { max_attempts: 2, ..crate::api::test_support::fast_watchdog() }).await;
    rig.parts.cache.put(&cache_key(&url_of(0)), cache::Kind::Stream, b"bytes").await.unwrap();
    let (slot, player) = playing(&rig, video(&url_of(0))).await;
    rig.parts.stage.ready(slot as usize, rig.parts.stage.generation_at(slot as usize));

    eventually(|| player.loads() >= 3).await;

    assert!(!rig.parts.cache.contains(&cache_key(&url_of(0))).await);
}

#[tokio::test]
async fn inv_api_06_a_foreground_pass_that_finds_the_proxy_down_is_retried_until_it_is_back() {
    let rig = rig_with(|config| config.watchdog = WatchdogConfig { foreground_delays_ms: vec![5; 400], ..crate::api::test_support::fast_watchdog() }).await;
    let (_, player) = playing(&rig, video(&url_of(0))).await;
    let plays = || player.calls().iter().filter(|call| **call == SlotCall::Play).count();
    let before = plays();
    rig.parts.proxy.stop().await;

    PoolControl::foreground(&rig.parts.stage);
    eventually(|| rig.parts.stage.foreground_passes() >= 3).await;
    assert_eq!(plays(), before);
    rig.parts.proxy.restart(0).await.unwrap();

    eventually(|| plays() > before).await;
}

#[tokio::test]
async fn inv_api_06_a_load_refused_by_a_dead_proxy_listener_rebinds_it_before_the_retry() {
    let rig = rig().await;
    let (slot, player) = playing(&rig, video(&url_of(0))).await;
    crate::proxy::test_client::close_listener_silently(rig.parts.proxy.port());
    assert!(!rig.parts.proxy.ping().await);

    rig.parts.stage.failed(slot as usize, rig.parts.stage.generation_at(slot as usize), 0, "Could not connect to the server".to_string());

    eventually(|| player.loads() >= 2).await;
    assert!(rig.parts.proxy.ping().await);
}

#[tokio::test]
async fn inv_api_06_returning_to_the_foreground_rebinds_a_listener_that_died_in_the_background() {
    let quiet = WatchdogConfig { first_window_ms: 600_000, progress_window_ms: 600_000, startup_delay_ms: 600_000, ..WatchdogConfig::default() };
    let rig = rig_with(|config| config.watchdog = quiet).await;
    playing(&rig, video(&url_of(0))).await;
    PoolControl::background(&rig.parts.stage);
    crate::proxy::test_client::close_listener_silently(rig.parts.proxy.port());
    assert!(!rig.parts.proxy.ping().await);

    PoolControl::foreground(&rig.parts.stage);

    tokio::time::timeout(std::time::Duration::from_secs(3), eventually_async(|| rig.parts.proxy.ping())).await.expect("the foreground never rebound the listener");
}

#[tokio::test]
async fn inv_api_13_a_slot_that_renders_ends_and_plays_again_moves_the_place_through_rendered_and_ended() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let (slot, player) = playing(&rig, once(&url_of(0))).await;
    stage.visible(once(&url_of(0)), true);
    let state = || rig.parts.places.state(&url_of(0));

    stage.ready(slot as usize, stage.generation_at(slot as usize));
    assert_eq!(state(), MediaState::Ready);
    stage.visual_ready(slot as usize, stage.generation_at(slot as usize));
    assert_eq!(state(), MediaState::Rendered);
    stage.ended(slot as usize, stage.generation_at(slot as usize));
    assert_eq!(state(), MediaState::Ended);
    stage.visual_ready(slot as usize, stage.generation_at(slot as usize));
    assert_eq!(state(), MediaState::Ended);
    stage.set_playing(&url_of(0), true, &[]);

    eventually(|| state() == MediaState::Rendered).await;
    eventually(|| player.calls().iter().filter(|call| **call == SlotCall::Play).count() == 2).await;
    let calls = player.calls();
    let tail: Vec<&SlotCall> = calls.iter().rev().take(3).rev().collect();
    assert_eq!(tail, [&SlotCall::Pause, &SlotCall::Seek { position_ms: 0 }, &SlotCall::Play], "the end stops, rewinds, and only then does the next play start");
}

#[tokio::test]
async fn inv_api_13_a_repeated_or_stale_ended_never_replaces_a_seek_made_after_the_end_and_forgets_the_old_position() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let (slot, player) = playing(&rig, once(&url_of(0))).await;
    let key = url_of(0);
    stage.visible(once(&key), true);
    let slot = slot as usize;
    let generation = stage.generation_at(slot);
    stage.ready(slot, generation);
    {
        let mut state = stage.state();
        state.positions.remember(&key, 900);
    }
    stage.note_seek(&key, 900);

    stage.ended(slot, generation);
    let (remembered, hinted) = (stage.state().positions.take(&key), stage.inner.seeks.lock().unwrap().contains_key(&key));
    assert_eq!((remembered, hinted), (None, false));
    stage.seek(&key, 1_200);
    stage.ended(slot, generation);
    stage.ended(slot, generation - 1);

    eventually(|| player.calls().last() == Some(&SlotCall::Seek { position_ms: 1_200 })).await;
    assert_eq!(player.calls().iter().filter(|call| **call == SlotCall::Seek { position_ms: 0 }).count(), 1);
    assert_eq!(rig.parts.places.state(&key), MediaState::Ended);
}

#[tokio::test]
async fn inv_api_13_a_looping_source_never_ends_the_place_or_stops_the_playback() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let (slot, player) = playing(&rig, video(&url_of(0))).await;
    stage.visible(video(&url_of(0)), true);
    let slot = slot as usize;
    stage.ready(slot, stage.generation_at(slot));
    let before = player.calls();

    stage.ended(slot, stage.generation_at(slot));

    assert_eq!(rig.parts.places.state(&url_of(0)), MediaState::Ready);
    assert_eq!(player.calls(), before);
}

fn quick_trim(config: &mut crate::api::Assembly) {
    config.pool = crate::api::PoolSettings { trim_delay_ms: 300, ..crate::api::PoolSettings::default() };
}

#[tokio::test]
async fn inv_idle_03_a_pending_trim_wakes_the_stage_at_its_deadline_and_not_before() {
    let rig = rig_with(quick_trim).await;
    let stage = &rig.parts.stage;
    stage.visible(video(&url_of(0)), true);
    eventually(|| rig.factory.slot_holding(&url_of(0)).is_some()).await;
    let (_, slot) = rig.holder(&url_of(0));
    let armed = stage.wakes();

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(stage.wakes(), armed, "no wake before the trim deadline");
    assert!(!slot.calls().contains(&SlotCall::Unload));

    tokio::time::timeout(std::time::Duration::from_secs(3), eventually(|| slot.calls().contains(&SlotCall::Unload))).await.expect("the trim never came");
    assert!(stage.wakes() > armed, "the deadline woke the stage");
}

#[tokio::test(start_paused = true)]
async fn inv_idle_04_a_wanted_key_without_a_slot_wakes_nothing_until_an_event_gives_it_room() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let url = url_of(0);
    stage.replace_window(0, vec![Some(video(&url))]);
    stage.visible(video(&url), true);
    stage.set_playing(&url, true, &[]);
    eventually(|| rig.factory.slot_holding(&url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))).await;
    let (_, slot) = rig.holder(&url);

    stage.set_step(crate::memory::Step::PlaceholdersOnly);
    let directives = stage.release_under(rig.parts.clock.now_ms(), crate::pool::Pressure::Critical);
    stage.apply(directives);
    eventually(|| slot.calls().last() == Some(&SlotCall::Unload)).await;
    for _ in 0..4 {
        tokio::time::advance(std::time::Duration::from_millis(50)).await;
    }
    let settled = stage.wakes();

    for _ in 0..200 {
        tokio::time::advance(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(stage.wakes(), settled, "10 s with a wanted key that has no slot and no event");
    assert!(rig.factory.slot_holding(&url).is_none(), "nothing gave the key room");

    stage.set_step(crate::memory::Step::Normal);
    eventually(|| rig.factory.slot_holding(&url).is_some()).await;
}

#[tokio::test]
async fn inv_idle_04_a_foreground_restores_every_saved_slot_without_another_event() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(1, videos(3));
    eventually(|| (0..3).all(|index| rig.factory.slot_holding(&url_of(index)).is_some())).await;

    let directives = PoolControl::background(stage);
    stage.apply(directives);
    eventually(|| (0..3).filter(|index| rig.factory.slot_holding(&url_of(*index)).is_some()).count() == 0).await;
    PoolControl::foreground(stage);

    eventually(|| (0..3).all(|index| rig.factory.slot_holding(&url_of(index)).is_some())).await;
}

fn idle_watchdog(config: &mut crate::api::Assembly) {
    const HOUR: u64 = 3_600_000;
    config.watchdog = WatchdogConfig {
        first_window_ms: HOUR,
        progress_window_ms: HOUR,
        cooldown_ms: HOUR,
        startup_delay_ms: HOUR,
        retry_delays_ms: vec![HOUR; 5],
        foreground_delays_ms: vec![HOUR; 3],
        ..WatchdogConfig::default()
    };
}

async fn silence(stage: &crate::api::Stage) -> u64 {
    let before = stage.wakes();
    for _ in 0..200 {
        tokio::time::advance(std::time::Duration::from_millis(50)).await;
    }
    stage.wakes() - before
}

async fn settle() {
    for _ in 0..40 {
        tokio::time::advance(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test(start_paused = true)]
async fn inv_idle_04_a_key_refused_by_the_ledger_returns_when_the_ceiling_frees_room() {
    let rig = rig_with(idle_watchdog).await;
    let stage = &rig.parts.stage;
    stage.replace_window(0, videos(1));
    eventually(|| rig.factory.slot_holding(&url_of(0)).is_some()).await;
    rig.parts.ledger.set_ceiling(rig.parts.ledger.used());

    stage.visible(video(&url_of(1)), true);
    stage.set_playing(&url_of(1), true, &[]);
    settle().await;
    assert!(rig.factory.slot_holding(&url_of(1)).is_none(), "the ledger refused the slot");
    assert_eq!(silence(stage).await, 0, "10 s of silence after the refusal");

    rig.parts.ledger.set_ceiling(rig.parts.ledger.used() * 4);
    eventually(|| rig.factory.slot_holding(&url_of(1)).is_some()).await;
}

async fn crowded_for_slots(event: impl FnOnce(&crate::api::Stage)) {
    let rig = rig_with(|config| {
        idle_watchdog(config);
        config.pool = crate::api::PoolSettings { reach: 0, ..crate::api::PoolSettings::default() };
        config.defaults.slots = 2;
    })
    .await;
    let stage = &rig.parts.stage;
    stage.pin(&url_of(0));
    stage.pin(&url_of(1));
    eventually(|| rig.factory.slot_holding(&url_of(0)).is_some() && rig.factory.slot_holding(&url_of(1)).is_some()).await;

    stage.visible(video(&url_of(2)), true);
    stage.set_playing(&url_of(2), true, &[]);
    settle().await;
    assert!(rig.factory.slot_holding(&url_of(2)).is_none(), "every slot is pinned, slots={}", stage.slot_count());
    assert_eq!(silence(stage).await, 0, "10 s of silence after the refusal");

    event(stage);
    eventually(|| rig.factory.slot_holding(&url_of(2)).is_some()).await;
}

#[tokio::test(start_paused = true)]
async fn inv_idle_04_a_key_refused_for_slots_returns_when_an_unpin_makes_a_spare() {
    crowded_for_slots(|stage| stage.unpin(&url_of(0))).await;
}

#[tokio::test(start_paused = true)]
async fn inv_idle_04_a_key_refused_for_slots_returns_when_the_other_pin_is_released() {
    crowded_for_slots(|stage| stage.unpin(&url_of(1))).await;
}

#[tokio::test]
async fn inv_idle_04_a_trim_tick_between_a_refusal_and_the_freed_room_keeps_the_wait_for_room() {
    let rig = rig_with(|config| {
        idle_watchdog(config);
        config.pool = crate::api::PoolSettings { reach: 1, trim_delay_ms: 300, ..crate::api::PoolSettings::default() };
        config.defaults.slots = 5;
    })
    .await;
    let stage = &rig.parts.stage;
    stage.pin(&url_of(9));
    stage.replace_window(1, videos(3));
    eventually(|| (0..3).all(|index| rig.factory.slot_holding(&url_of(index)).is_some()) && rig.factory.slot_holding(&url_of(9)).is_some()).await;
    rig.parts.ledger.set_ceiling(rig.parts.ledger.used().saturating_sub(1));

    stage.visible(video(&url_of(7)), true);
    stage.set_playing(&url_of(7), true, &[]);
    let (neighbour, _) = rig.holder(&url_of(0));
    let level = || stage.state().ring.retention_of(neighbour as usize);
    let before = level();
    tokio::time::timeout(std::time::Duration::from_secs(3), eventually(|| level() != before))
        .await
        .unwrap_or_else(|_| panic!("the trim tick never came: the neighbour stayed {before:?}"));
    assert!(rig.factory.slot_holding(&url_of(7)).is_none(), "the ledger kept the wanted key out");

    rig.parts.ledger.set_ceiling(rig.parts.ledger.used() * 4);
    tokio::time::timeout(std::time::Duration::from_secs(3), eventually(|| rig.factory.slot_holding(&url_of(7)).is_some()))
        .await
        .expect("the freed room never brought the key back");
}

#[tokio::test(start_paused = true)]
async fn inv_idle_04_a_ledger_release_that_leaves_the_room_unchanged_does_not_start_an_attempt() {
    let rig = rig_with(idle_watchdog).await;
    let stage = &rig.parts.stage;
    let ledger = &rig.parts.ledger;
    stage.replace_window(0, videos(1));
    eventually(|| rig.factory.slot_holding(&url_of(0)).is_some()).await;
    ledger.set_ceiling(ledger.used() + 10);
    let early = ledger.reserve(crate::memory::Owner::ImageDecode, 4).unwrap();

    stage.visible(video(&url_of(1)), true);
    stage.set_playing(&url_of(1), true, &[]);
    settle().await;
    assert!(rig.factory.slot_holding(&url_of(1)).is_none(), "the ledger refused the slot");
    assert_eq!(silence(stage).await, 0, "10 s of silence after the refusal");

    let late = ledger.reserve(crate::memory::Owner::ImageDecode, 4).unwrap();
    drop(early);
    assert_eq!(silence(stage).await, 1, "one freed wake and no attempt: the room is the same as at the refusal");

    drop(late);
    ledger.set_ceiling(ledger.used() * 4);
    eventually(|| rig.factory.slot_holding(&url_of(1)).is_some()).await;
}

#[tokio::test]
async fn inv_api_43_a_late_event_of_an_older_load_does_not_touch_the_new_load_of_the_slot() {
    type Case = (&'static str, fn(&super::Stage, usize, u64));
    let events: [Case; 3] = [
        ("ready", |stage, slot, generation| stage.ready(slot, generation)),
        ("visual_ready", |stage, slot, generation| stage.visual_ready(slot, generation)),
        ("ended", |stage, slot, generation| stage.ended(slot, generation)),
    ];
    for (name, late) in events {
        let rig = rig().await;
        let stage = &rig.parts.stage;
        stage.replace_window(0, videos(3));
        stage.visible(video(&url_of(0)), true);
        eventually(|| rig.factory.slot_holding(&url_of(0)).is_some()).await;
        let (slot, _) = rig.holder(&url_of(0));
        let slot = slot as usize;
        let old = stage.generation_at(slot);
        let id = stage.load_id_at(slot).unwrap();
        stage.reload(&id, false);
        assert_ne!(stage.generation_at(slot), old, "{name}: the reload kept the generation");
        let before = rig.parts.places.state(&url_of(0));

        late(stage, slot, old);

        assert_eq!(rig.parts.places.state(&url_of(0)), before, "{name} of the old load changed the new load");
    }
}

#[tokio::test]
async fn inv_api_44_an_action_held_across_a_reload_of_the_same_key_does_not_touch_the_new_load() {
    for (name, action) in [("recover", crate::session::Action::Recover), ("purge", crate::session::Action::PurgeCache(url_of(0)))] {
        let rig = rig().await;
        let (slot, player) = playing(&rig, video(&url_of(0))).await;
        let (stage, routes) = (&rig.parts.stage, &rig.parts.routes);
        let slot = slot as usize;
        let old = stage.load_id_at(slot).unwrap();
        routes.hold_repair_and_purge(true);
        stage.perform(old, action);
        eventually(|| routes.barrier_entries() == 1).await;
        let current = stage.load_id_at(slot).unwrap();
        stage.reload(&current, false);
        eventually(|| player.loads() == 2 && stage.commands_idle()).await;
        let (calls, generation) = (player.calls(), stage.generation_at(slot));

        routes.hold_repair_and_purge(false);
        stage.inner.action_completed.notified().await;
        eventually(|| stage.commands_idle()).await;

        assert_eq!(player.calls(), calls, "{name} held across the reload sent commands to the new load");
        assert_eq!(stage.generation_at(slot), generation, "{name} held across the reload reloaded the new load again");
    }
}

#[tokio::test]
async fn inv_api_44_a_stale_action_changes_nothing_of_the_new_load() {
    use crate::session::Action;
    let rig = rig().await;
    let (slot, player) = playing(&rig, video(&url_of(0))).await;
    let stage = &rig.parts.stage;
    let slot = slot as usize;
    let old = stage.load_id_at(slot).unwrap();
    let current = stage.load_id_at(slot).unwrap();
    stage.reload(&current, false);
    eventually(|| player.loads() == 2 && stage.commands_idle()).await;
    let snapshot = || (stage.generation_at(slot), stage.armed_generation(&url_of(0)), rig.parts.places.state(&url_of(0)), stage.foreground_passes());
    let before = snapshot();
    assert_eq!(before.1, Some(before.0), "the new load did not arm its own timer");
    let calls = player.calls();

    let actions = [
        Action::Replay,
        Action::Disarm,
        Action::Arm { delay_ms: 5 },
        Action::Retry(5),
        Action::ForegroundRetry(5),
        Action::Fail("late".to_string()),
        Action::RefreshRoute,
        Action::ForegroundPass,
    ];
    for action in actions {
        let name = format!("{action:?}");
        stage.perform(old.clone(), action);
        assert_eq!(snapshot(), before, "a stale {name} changed the new load");
    }
    eventually(|| stage.commands_idle()).await;
    assert_eq!(player.calls(), calls, "a stale action sent commands to the new load");
}

#[tokio::test]
async fn inv_api_18_a_note_that_ended_and_was_rewound_before_its_place_closed_returns_at_the_start() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let key = url_of(0);
    stage.visible(once(&key), true);
    stage.set_playing(&key, true, &[]);
    eventually(|| rig.factory.slot_holding(&key).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))).await;
    let (slot, player) = rig.holder(&key);
    let slot = slot as usize;
    let generation = stage.generation_at(slot);
    stage.ready(slot, generation);
    {
        let mut sample = player.sample.lock().unwrap();
        sample.position_ms = 60_000;
        sample.duration_ms = Some(60_000);
    }

    stage.ended(slot, generation);
    stage.seek_fraction(&key, 0.0);
    stage.set_playing(&key, false, &[]);
    stage.visible(once(&key), false);
    eventually(|| rig.factory.slot_holding(&key).is_none()).await;
    stage.visible(once(&key), true);

    eventually(|| rig.factory.slot_holding(&key).is_some() && stage.commands_idle()).await;
    let calls = rig.holder(&key).1.calls();
    let load = calls.iter().rposition(|call| matches!(call, SlotCall::Load { .. })).unwrap();
    assert!(!calls[load..].iter().any(|call| matches!(call, SlotCall::Seek { position_ms } if *position_ms > 0)), "{calls:?}");
}
