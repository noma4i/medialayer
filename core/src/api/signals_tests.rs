use super::test_support::{eventually, shared, unload, FakeDirectives, FakePool, FakePrefetch, FakeRelease};
use super::*;
use crate::cache::SystemClock;
use crate::memory::{Ledger, Step};
use crate::pool::{Command, Directive};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::runtime::Handle;

struct Rig {
    pool: Arc<FakePool>,
    prefetch: Arc<FakePrefetch>,
    directives: Arc<FakeDirectives>,
    release: Arc<FakeRelease>,
    signals: Signals,
    ledger: Ledger,
}

fn rig() -> Rig {
    let pool: Arc<FakePool> = shared();
    let prefetch: Arc<FakePrefetch> = shared();
    let directives: Arc<FakeDirectives> = shared();
    let release: Arc<FakeRelease> = shared();
    let ledger = Ledger::new(1 << 30);
    let signals = Signals::new(
        &Handle::current(),
        SignalPorts {
            pool: pool.clone(),
            prefetch: prefetch.clone(),
            directives: directives.clone(),
            release: release.clone(),
            suspendable: Vec::new(),
            ledger: ledger.clone(),
            clock: Arc::new(SystemClock),
        },
    );
    Rig { pool, prefetch, directives, release, signals, ledger }
}

#[tokio::test]
async fn inv_heal_10_the_memory_step_recovers_after_the_ledger_frees() {
    let rig = rig();
    let held = rig.ledger.reserve(crate::memory::Owner::ImageDecode, 950_000_000).unwrap();
    rig.signals.memory_warning(Pressure::Critical);
    eventually(|| !rig.pool.steps.lock().unwrap().is_empty()).await;
    drop(held);
    eventually(|| rig.pool.steps.lock().unwrap().last() == Some(&Step::Normal)).await;
}

#[tokio::test(start_paused = true)]
async fn inv_heal_10_the_memory_step_recovers_after_warning_decay() {
    let rig = rig();
    rig.signals.memory_warning(Pressure::Critical);
    eventually(|| rig.pool.steps.lock().unwrap().last() == Some(&Step::ShorterBuffer)).await;
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    eventually(|| rig.pool.steps.lock().unwrap().last() == Some(&Step::Normal)).await;
}

fn events(rig: &Rig) -> Vec<String> {
    rig.pool.events.lock().unwrap().clone()
}

#[tokio::test]
async fn inv_api_05_memory_warning_lowers_the_step_and_releases_slots_by_level() {
    for (level, step, event) in [
        (Pressure::Moderate, Step::LowerQuality, "release_under Moderate"),
        (Pressure::Critical, Step::ShorterBuffer, "release_under Critical"),
    ] {
        let rig = rig();
        rig.signals.memory_warning(level);
        eventually(|| !events(&rig).is_empty()).await;
        assert_eq!(events(&rig), vec![event.to_string()]);
        assert_eq!(*rig.pool.steps.lock().unwrap(), vec![step]);
    }
}

#[tokio::test]
async fn inv_api_05_pressure_and_background_stop_prefetch_drop_the_platform_memory_caches_and_forward_the_slot_directives() {
    for trigger in [|signals: &Signals| signals.memory_warning(Pressure::Critical), |signals: &Signals| signals.background()] {
        let rig = rig();
        *rig.pool.to_return.lock().unwrap() = vec![Directive::StopPrefetch, unload(1), Directive::DropMemoryCache];

        trigger(&rig.signals);

        eventually(|| !rig.directives.applied.lock().unwrap().is_empty() && rig.release.drops.load(Ordering::SeqCst) == 1).await;
        assert_eq!(rig.prefetch.stops.load(Ordering::SeqCst), 1);
        let applied = rig.directives.applied.lock().unwrap();
        assert!(matches!(applied.as_slice(), [batch] if matches!(batch.as_slice(), [Directive::Slot(Command::Unload { slot: 1 })])));
    }
}

#[tokio::test]
async fn inv_api_05_foreground_after_pressure_returns_the_step_to_normal() {
    let rig = rig();

    rig.signals.memory_warning(Pressure::Critical);
    rig.signals.foreground();

    eventually(|| events(&rig).contains(&"foreground".to_string())).await;
    assert_eq!(rig.pool.steps.lock().unwrap().last(), Some(&Step::Normal));
}

#[tokio::test]
async fn inv_api_05_signals_apply_in_the_order_they_arrive() {
    let rig = rig();

    rig.signals.background();
    rig.signals.foreground();
    rig.signals.memory_warning(Pressure::Moderate);

    eventually(|| events(&rig).len() == 3).await;
    assert_eq!(events(&rig), vec!["background", "foreground", "release_under Moderate"]);
}

#[tokio::test]
async fn inv_api_05_an_overflowing_queue_keeps_the_worst_pressure_and_the_last_lifecycle_signal() {
    let rig = rig();

    rig.signals.memory_warning(Pressure::Critical);
    for _ in 0..100 {
        rig.signals.memory_warning(Pressure::Moderate);
    }
    rig.signals.background();
    rig.signals.foreground();

    eventually(|| events(&rig).last() == Some(&"foreground".to_string())).await;
    let seen = events(&rig);
    assert!(seen.contains(&"release_under Critical".to_string()), "{seen:?}");
    assert!(seen.len() < 70, "{seen:?}");
}
