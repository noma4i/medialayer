use crate::api::ports::{PoolControl, SlotDirectives as _};
use crate::api::slots::SlotCall;
use crate::api::source::MediaItem;
use crate::api::test_support::{eventually, rig, rig_with, url_of, video, videos, Rig};
use crate::api::types::{Kind, MediaState};
use crate::identity::cache_key;
use crate::cache;
use crate::session::WatchdogConfig;
use std::sync::{Arc, Mutex};

const HLS: &str = "https://o.test/hls/master.m3u8";

fn hls() -> MediaItem {
    MediaItem::bare(HLS, Kind::Video)
}

async fn playing(rig: &Rig, source: MediaItem) -> (u32, Arc<crate::api::test_support::MockSlot>) {
    let url = source.url.clone();
    rig.parts.stage.replace_window(0, vec![Some(source)]);
    rig.parts.stage.set_playing(&url, true);
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

    stage.ready(slot as usize);
    assert_eq!(rig.parts.places.state(&url_of(0)), MediaState::Ready);
    stage.visual_ready(slot as usize);

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

    rig.parts.stage.failed(slot as usize, 404, String::new());

    eventually(|| player.loads() == 2).await;
    assert_eq!(player.calls().iter().filter(|call| **call == SlotCall::Unload).count(), 1);
}

#[tokio::test]
async fn inv_api_06_repeated_playback_failures_reload_the_player_and_then_fail_the_place_with_the_reason() {
    let rig = rig().await;
    rig.parts.places.declared(&url_of(0), false);
    let (slot, player) = playing(&rig, video(&url_of(0))).await;

    for _ in 0..6 {
        rig.parts.stage.failed(slot as usize, 0, "decoder died".to_string());
    }

    assert_eq!(rig.parts.places.state(&url_of(0)), MediaState::Failed { detail: "decoder died".to_string() });
    eventually(|| player.loads() >= 2).await;
}

#[tokio::test]
async fn inv_api_06_a_silent_player_is_reloaded_by_the_watchdog() {
    let rig = rig().await;
    let (slot, silent) = playing(&rig, video(&url_of(0))).await;
    rig.parts.stage.ready(slot as usize);

    eventually(|| silent.loads() >= 2).await;
}

#[tokio::test]
async fn inv_api_06_a_player_whose_clock_moves_is_checked_again_and_left_alone() {
    let rig = rig_with(|_| {}).await;
    *rig.factory.advance_ms.lock().unwrap() = 1_000;
    let (slot, healthy) = playing(&rig, video(&url_of(0))).await;
    rig.parts.stage.ready(slot as usize);

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
    stage.stalled(idle_slot as usize, 0);
    assert_eq!(stage.armed_tasks(), 0);
    assert_eq!(idle.samples.load(std::sync::atomic::Ordering::SeqCst), 0);

    stage.set_playing(&url_of(0), true);
    eventually(|| rig.factory.slot_holding(&url_of(0)).is_some()).await;
    let (active_slot, active) = rig.holder(&url_of(0));
    stage.stalled(active_slot as usize, 0);
    assert_eq!(stage.armed_tasks(), 1);

    eventually(|| active.samples.load(std::sync::atomic::Ordering::SeqCst) >= 1).await;
}

#[tokio::test]
async fn inv_api_06_going_to_the_background_pauses_playback_and_returning_resumes_it() {
    let rig = rig().await;
    let (_, player) = playing(&rig, video(&url_of(0))).await;
    let stage = &rig.parts.stage;

    let directives = PoolControl::background(stage);
    stage.apply(directives);
    eventually(|| player.calls().last() == Some(&SlotCall::Pause)).await;
    PoolControl::foreground(stage);

    eventually(|| player.calls().iter().filter(|call| **call == SlotCall::Play).count() >= 2).await;
}

#[tokio::test]
async fn inv_api_06_watchdog_escalation_purges_the_cached_source_and_loads_it_again() {
    let rig = rig_with(|config| config.watchdog = WatchdogConfig { max_attempts: 2, ..crate::api::test_support::fast_watchdog() }).await;
    rig.parts.cache.put(&cache_key(&url_of(0)), cache::Kind::Stream, b"bytes").await.unwrap();
    let (slot, player) = playing(&rig, video(&url_of(0))).await;
    rig.parts.stage.ready(slot as usize);

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

    rig.parts.stage.failed(slot as usize, 0, "Could not connect to the server".to_string());

    eventually(|| player.loads() >= 2).await;
    assert!(rig.parts.proxy.ping().await);
}

#[tokio::test]
async fn inv_api_06_returning_to_the_foreground_rebinds_a_listener_that_died_in_the_background() {
    let rig = rig().await;
    let (_, player) = playing(&rig, video(&url_of(0))).await;
    PoolControl::background(&rig.parts.stage);
    crate::proxy::test_client::close_listener_silently(rig.parts.proxy.port());
    let before = player.calls().iter().filter(|call| **call == SlotCall::Play).count();

    PoolControl::foreground(&rig.parts.stage);

    eventually(|| player.calls().iter().filter(|call| **call == SlotCall::Play).count() > before).await;
    assert!(rig.parts.proxy.ping().await);
}

#[tokio::test]
async fn inv_api_13_a_slot_that_renders_ends_and_plays_again_moves_the_place_through_rendered_and_ended() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    let (slot, _) = playing(&rig, video(&url_of(0))).await;
    stage.visible(video(&url_of(0)), true);
    let state = || rig.parts.places.state(&url_of(0));

    stage.ready(slot as usize);
    assert_eq!(state(), MediaState::Ready);
    stage.visual_ready(slot as usize);
    assert_eq!(state(), MediaState::Rendered);
    stage.ended(slot as usize);
    assert_eq!(state(), MediaState::Ended);
    stage.visual_ready(slot as usize);
    assert_eq!(state(), MediaState::Ended);
    stage.set_playing(&url_of(0), false);
    stage.set_playing(&url_of(0), true);

    eventually(|| state() == MediaState::Rendered).await;
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
    stage.set_playing(&url, true);
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
