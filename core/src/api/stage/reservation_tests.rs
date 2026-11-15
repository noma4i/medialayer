use crate::api::ports::PoolControl;
use crate::api::slots::{SlotCall, SlotRunner};
use crate::api::source::PlaybackRoute;
use crate::api::test_support::{eventually, rig, rig_with, url_of, video, videos, MockFactory, Rig};
use crate::memory::{Held, Ledger, Owner};
use crate::pool::Pressure;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::runtime::Handle;

const SLOT_BYTES: u64 = 24 * 1024 * 1024;

fn load(key: &str, generation: u64) -> SlotCall {
    SlotCall::Load {
        key: key.to_string(),
        route: PlaybackRoute { url: format!("https://{key}"), headers: Default::default(), generation: Some(0), hls: false, looping: true },
        generation,
    }
}

fn reserved(ledger: &Ledger, slot: usize) -> u64 {
    ledger.used_by(Owner::Slot(slot))
}

fn unloads(factory: &MockFactory) -> usize {
    factory.slots.lock().unwrap().values().map(|slot| slot.calls().iter().filter(|call| **call == SlotCall::Unload).count()).sum()
}

#[tokio::test]
async fn inv_pool_17_native_unload_ack_gates_the_reservation_release() {
    let rig = rig().await;
    let (stage, ledger) = (&rig.parts.stage, &rig.parts.ledger);
    stage.replace_window(0, videos(1));
    eventually(|| rig.factory.live_sources() == 1).await;
    let held = reserved(ledger, 0);
    assert!(held > 0, "no slot reservation exists after the Load was applied");
    ledger.set_ceiling(ledger.used());
    assert!(ledger.reserve(Owner::ProxyBuffer, held).is_err(), "harness invalid: the ledger admits a second owner while the slot is live");

    let gate = rig.factory.unload_gate.hold();
    stage.release_all();
    eventually(|| rig.factory.unload_gate.entered() == 1).await;
    assert_eq!(rig.factory.live_sources(), 1, "the native source was not live while Unload was held");
    assert!(ledger.reserve(Owner::ProxyBuffer, held).is_err(), "another owner reserved the slot's bytes before the native Unload was acked");

    drop(gate);
    eventually(|| rig.factory.live_sources() == 0).await;
    eventually(|| ledger.reserve(Owner::ProxyBuffer, held).is_ok()).await;
}

#[derive(Debug, Clone, Copy)]
enum Cause {
    SourceChange,
    Explicit,
    ReleaseAll,
    Pressure,
    Background,
    Timer,
    Shrink,
    Close,
}

impl Cause {
    fn watched(self) -> Vec<usize> {
        match self {
            Cause::Explicit => vec![0],
            Cause::Background => vec![3],
            _ => vec![3, 4],
        }
    }
}

async fn loaded(rig: &Rig, cause: Cause) {
    let stage = &rig.parts.stage;
    if let Cause::Explicit = cause {
        stage.visible(video(&url_of(0)), true);
        eventually(|| rig.factory.live_sources() == 1).await;
    } else {
        stage.set_slots(5);
        stage.replace_window(4, videos(5));
        eventually(|| rig.factory.live_sources() == 2).await;
    }
}

async fn release_by(rig: &Rig, cause: Cause) -> Option<tokio::task::JoinHandle<()>> {
    let stage = &rig.parts.stage;
    match cause {
        Cause::SourceChange => stage.replace_window(0, videos(1)),
        Cause::Explicit => stage.visible(video(&url_of(0)), false),
        Cause::ReleaseAll => stage.release_all(),
        Cause::Pressure => {
            stage.release_under(0, Pressure::Critical);
        }
        Cause::Background => {
            stage.background();
        }
        Cause::Timer => {}
        Cause::Shrink => stage.set_slots(3),
        Cause::Close => {
            let closing = stage.clone();
            return Some(tokio::spawn(async move { closing.close().await }));
        }
    }
    None
}

#[tokio::test]
async fn inv_pool_17_every_release_cause_keeps_the_reservation_until_the_native_ack() {
    for cause in [Cause::SourceChange, Cause::Explicit, Cause::ReleaseAll, Cause::Pressure, Cause::Background, Cause::Timer, Cause::Shrink, Cause::Close] {
        let rig = rig_with(|config| {
            if let Cause::Timer = cause {
                config.pool.trim_delay_ms = 30;
            }
        })
        .await;
        let gate = if matches!(cause, Cause::Close | Cause::Shrink) { rig.factory.dispose_gate.hold() } else { rig.factory.unload_gate.hold() };
        loaded(&rig, cause).await;
        let (ledger, watched) = (&rig.parts.ledger, cause.watched());
        let closing = release_by(&rig, cause).await;
        let entered = if matches!(cause, Cause::Close | Cause::Shrink) { &rig.factory.dispose_gate } else { &rig.factory.unload_gate };
        eventually(|| entered.entered() >= watched.len()).await;

        for slot in &watched {
            assert_eq!(reserved(ledger, *slot), SLOT_BYTES, "{cause:?}: the reservation of slot {slot} left before the native ack");
        }
        ledger.set_ceiling(ledger.used());
        assert!(ledger.reserve(Owner::ProxyBuffer, SLOT_BYTES).is_err(), "{cause:?}: another owner was admitted before the native ack");

        drop(gate);
        if let Some(closing) = closing {
            closing.await.unwrap();
        }
        eventually(|| watched.iter().all(|slot| reserved(ledger, *slot) == 0)).await;
        assert!(ledger.reserve(Owner::ProxyBuffer, SLOT_BYTES).is_ok(), "{cause:?}: the bytes did not return after the native ack");
    }
}

#[tokio::test]
async fn inv_pool_19_a_reload_keeps_its_admission_and_a_new_source_gets_its_own() {
    let rig = rig().await;
    let (stage, ledger) = (&rig.parts.stage, &rig.parts.ledger);
    stage.set_slots(5);
    stage.replace_window(4, videos(5));
    eventually(|| rig.factory.live_sources() == 2).await;

    let gate = rig.factory.unload_gate.hold();
    stage.reload_key(&url_of(4));
    eventually(|| rig.factory.unload_gate.entered() == 1).await;
    assert_eq!(reserved(ledger, 4), SLOT_BYTES, "a reload doubled or dropped the admission of its slot");
    drop(gate);
    eventually(|| rig.factory.slot(4).loads() == 2).await;
    assert_eq!(reserved(ledger, 4), SLOT_BYTES);

    let gate = rig.factory.unload_gate.hold();
    stage.replace_window(4, (0..5).map(|index| Some(video(&format!("https://o.test/w{index}.mp4")))).collect());
    eventually(|| rig.factory.unload_gate.entered() == 3).await;
    assert_eq!(reserved(ledger, 4), 2 * SLOT_BYTES, "the new source did not get its own admission next to the unloading one");
    drop(gate);
    eventually(|| reserved(ledger, 4) == SLOT_BYTES).await;
}

#[tokio::test(start_paused = true)]
async fn inv_pool_20_a_refused_page_waits_for_the_ack_and_the_window_refills_without_any_input() {
    let mut rig = rig().await;
    let pool = crate::api::PoolSettings { trim_delay_ms: 600_000, ..crate::api::PoolSettings::default() };
    crate::api::test_support::replace_stage_clock(&mut rig, pool, Arc::new(crate::api::test_support::TokioClock::new()));
    let (stage, ledger) = (&rig.parts.stage, &rig.parts.ledger);
    ledger.set_ceiling(SLOT_BYTES);
    stage.replace_window(0, videos(4));
    eventually(|| rig.factory.live_sources() == 1).await;

    let gate = rig.factory.unload_gate.hold();
    stage.move_window(1);
    eventually(|| rig.factory.unload_gate.entered() == 1).await;
    eventually(|| stage.state().is_room_wanted).await;
    assert_eq!(ledger.used(), SLOT_BYTES, "the ceiling was exceeded while the old source was still native-live");
    assert!(rig.factory.slot_holding(&url_of(1)).is_none(), "the nearer page loaded before the old source was acked");

    drop(gate);
    eventually(|| rig.factory.slot_holding(&url_of(1)).is_some()).await;
    assert_eq!(ledger.used(), SLOT_BYTES);
    assert_eq!(unloads(&rig.factory), 1, "the refill evicted more than the one source that made room");

    eventually(|| rig.factory.live_sources() == 1 && stage.commands_idle() && !stage.state().is_restore_requested
        && stage.inner.tick_wait.lock().unwrap().is_some_and(|due| due != Some(0) && due == *stage.inner.due.borrow())).await;
    let wakes = stage.wakes();
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    assert_eq!(stage.wakes(), wakes, "the settled pool kept waking");
}

#[tokio::test]
async fn inv_pool_18_a_cancelled_load_gives_its_reservation_back_and_a_live_source_keeps_its_unload() {
    let ledger = Ledger::unbounded();
    let factory: Arc<MockFactory> = Arc::default();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));
    let take = |bytes| Held::new(ledger.reserve(Owner::Slot(0), bytes).unwrap());
    runner.send_held(0, load("a", 1), Some(take(10)));
    eventually(|| runner.is_idle()).await;
    assert_eq!(ledger.used(), 10);

    let seek = factory.seek_gate.hold();
    runner.send(0, SlotCall::Seek { position_ms: 1 });
    eventually(|| factory.seek_gate.entered() == 1).await;
    runner.send(0, SlotCall::Unload);
    runner.send_held(0, load("b", 2), Some(take(20)));
    for _ in 0..40 {
        runner.send(0, SlotCall::Preroll);
        runner.send(0, SlotCall::Play);
        runner.send(0, SlotCall::Pause);
    }
    assert!(runner.queued(0) <= crate::api::lane::LANE_CAPACITY);
    runner.send_held(0, load("c", 3), Some(take(40)));
    assert_eq!(ledger.used(), 50, "the cancelled load kept its reservation: live a (10) and pending c (40) only");

    drop(seek);
    eventually(|| runner.is_idle()).await;
    let calls = factory.slot(0).calls();
    let unload = calls.iter().position(|call| *call == SlotCall::Unload).expect("the folded queue lost the unload of the live source");
    let next = calls.iter().position(|call| *call == load("c", 3)).expect("the last load never ran");
    assert!(unload < next, "{calls:?}");
    assert_eq!(ledger.used(), 40, "the unload ack did not release the old source");
}

#[tokio::test]
async fn inv_pool_21_dispose_follows_the_started_work_and_never_creates_for_a_load_that_did_not_start() {
    let ledger = Ledger::unbounded();
    let take = |bytes| Held::new(ledger.reserve(Owner::Slot(0), bytes).unwrap());

    let factory: Arc<MockFactory> = Arc::default();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));
    runner.send_held(0, load("never", 1), Some(take(5)));
    runner.dispose(0);
    runner.disposed().await;
    assert!(factory.created.lock().unwrap().is_empty(), "a player was created for a load that never started");
    assert_eq!(ledger.used(), 0);

    let started = factory.gate.hold();
    runner.send_held(1, load("started", 1), Some(take(7)));
    eventually(|| factory.gate.entered() == 1).await;
    runner.dispose(1);
    runner.send(3, SlotCall::Mute);
    eventually(|| factory.slots.lock().unwrap().get(&3).is_some_and(|slot| slot.calls().contains(&SlotCall::Mute))).await;
    assert_eq!(factory.slot(1).disposed.load(Ordering::SeqCst), 0, "dispose overtook the load that had started");
    drop(started);
    runner.disposed().await;
    assert_eq!(factory.slot(1).calls(), vec![load("started", 1)]);
    assert_eq!((factory.slot(1).disposed.load(Ordering::SeqCst), ledger.used()), (1, 0));

    let creating = factory.create_gate.hold();
    runner.send_held(2, load("created", 1), Some(take(9)));
    eventually(|| factory.create_gate.entered() == 3).await;
    runner.dispose(2);
    drop(creating);
    runner.disposed().await;
    assert_eq!(factory.slot(2).disposed.load(Ordering::SeqCst), 1, "the player made during the dispose was dropped without dispose");
    assert_eq!(ledger.used(), 0);

    let first = factory.slot(1);
    runner.send_held(1, load("again", 2), Some(take(3)));
    eventually(|| factory.slot(1).loads() == 1 && !Arc::ptr_eq(&factory.slot(1), &first)).await;
    assert_eq!(factory.created.lock().unwrap().iter().filter(|slot| **slot == 1).count(), 2, "the slot number was not given a new player");
    assert_eq!(first.disposed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn inv_pool_21_shrinking_disposes_the_removed_players_and_closing_leaves_no_slot() {
    let rig = rig().await;
    let (stage, ledger) = (&rig.parts.stage, &rig.parts.ledger);
    stage.set_slots(5);
    stage.replace_window(4, videos(5));
    eventually(|| rig.factory.live_sources() == 2).await;

    stage.set_slots(3);
    eventually(|| rig.factory.disposed_slots() == 2).await;
    assert_eq!(rig.factory.live_sources(), 0);
    assert_eq!((reserved(ledger, 3), reserved(ledger, 4)), (0, 0));

    stage.set_slots(5);
    stage.replace_window(4, videos(5));
    eventually(|| rig.factory.live_sources() == 2).await;
    assert_eq!(rig.factory.created.lock().unwrap().iter().filter(|slot| **slot == 4).count(), 2, "the regrown slot reused the destroyed player");

    for _ in 0..5 {
        stage.set_slots(3);
        stage.set_slots(5);
        stage.replace_window(4, videos(5));
        eventually(|| rig.factory.live_sources() == 2).await;
    }
    stage.set_slots(3);
    eventually(|| stage.slot_records() == 0).await;
    stage.set_slots(5);
    stage.replace_window(4, videos(5));
    eventually(|| rig.factory.live_sources() == 2).await;
    assert_eq!(stage.slot_records(), 2, "finished slot records stayed in the registry until the close");

    stage.close().await;
    assert_eq!(rig.factory.disposed_slots(), rig.factory.created.lock().unwrap().len());
    assert_eq!(ledger.used(), 0);
    let created = rig.factory.created.lock().unwrap().len();
    stage.replace_window(0, videos(3));
    tokio::time::timeout(std::time::Duration::from_secs(20), rig.parts.prefetcher.wait_live_at_most(0)).await.unwrap();
    assert_eq!(rig.factory.created.lock().unwrap().len(), created, "a declaration after the close made a player");
    assert_eq!(ledger.used(), 0, "a declaration after the close took a reservation");
}
