use crate::api::ports::PoolControl;
use crate::api::slots::SlotCall;
use crate::api::test_support::{eventually, rig, rig_with, url_of, video, videos, Rig};
use crate::session::{Action, WatchdogConfig};
use std::sync::Arc;

const HOUR_MS: u64 = 3_600_000;

fn slow_watchdog() -> WatchdogConfig {
    WatchdogConfig {
        first_window_ms: HOUR_MS,
        progress_window_ms: HOUR_MS,
        cooldown_ms: HOUR_MS,
        startup_delay_ms: HOUR_MS,
        retry_delays_ms: vec![HOUR_MS; 5],
        foreground_delays_ms: vec![HOUR_MS; 3],
        ..WatchdogConfig::default()
    }
}

async fn playing(rig: &Rig) -> usize {
    let url = url_of(0);
    rig.parts.stage.replace_window(0, videos(1));
    rig.parts.stage.set_playing(&url, true, &[]);
    eventually(|| rig.factory.slot_holding(&url).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))).await;
    rig.holder(&url).0 as usize
}

#[tokio::test]
async fn inv_api_46_waiting_work_holds_no_strong_stage_and_ends_with_its_load() {
    let rig = rig_with(|config| config.watchdog = slow_watchdog()).await;
    let (stage, routes) = (&rig.parts.stage, &rig.parts.routes);
    let routes_base = Arc::strong_count(routes);
    let holders = stage.holders();
    let slot = playing(&rig).await;
    let id = stage.load_id_at(slot).unwrap();
    routes.hold_repair_and_purge(true);

    stage.stalled(slot, id.generation, 0);
    stage.perform(id.clone(), Action::Recover);
    eventually(|| routes.barrier_entries() == 1).await;
    stage.foreground();
    eventually(|| routes.barrier_entries() == 2).await;

    assert!(stage.armed_tasks() >= 2 && stage.has_foreground());
    assert_eq!(stage.holders(), holders, "a timer, a repair or the foreground pass holds a strong stage while it waits");

    stage.release_all();
    assert_eq!(stage.armed_tasks(), 0, "the unload of the load left background work behind");
    eventually(|| Arc::strong_count(routes) == routes_base + 1).await;
    routes.hold_repair_and_purge(false);
}

#[tokio::test]
async fn inv_api_46_closing_cancels_every_background_work_and_a_finished_work_leaves_no_handle() {
    let rig = rig_with(|config| config.watchdog = slow_watchdog()).await;
    let (stage, routes) = (&rig.parts.stage, &rig.parts.routes);
    let routes_base = Arc::strong_count(routes);
    let slot = playing(&rig).await;
    let id = stage.load_id_at(slot).unwrap();

    stage.perform(id.clone(), Action::PurgeCache(url_of(0)));
    stage.inner.action_completed.notified().await;
    assert_eq!(stage.repairs_in_flight(), 0, "a finished purge kept its handle");

    routes.hold_repair_and_purge(true);
    stage.perform(id, Action::Recover);
    stage.foreground();
    eventually(|| routes.barrier_entries() == 2).await;
    assert_eq!(stage.repairs_in_flight(), 1);

    stage.close().await;

    assert_eq!((stage.armed_tasks(), stage.repairs_in_flight(), stage.has_foreground()), (0, 0, false));
    eventually(|| Arc::strong_count(routes) == routes_base).await;
    routes.hold_repair_and_purge(false);
    assert_eq!(rig.factory.slot(slot as u32).loads(), 1, "a cancelled recovery reloaded after the close");
}

#[tokio::test]
async fn inv_api_46_hiding_the_last_owner_cancels_the_cache_check_of_its_declaration() {
    let rig = rig().await;
    let stage = &rig.parts.stage;

    stage.visible(video(&url_of(0)), true);
    assert_eq!(stage.declaration_checks(), 1);
    stage.visible(video(&url_of(0)), false);

    assert_eq!(stage.declaration_checks(), 0, "the check of a retired declaration kept running");
}
