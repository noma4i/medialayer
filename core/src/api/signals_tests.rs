use super::test_support::{eventually, shared, unload, FakeDirectives, FakePool, FakePrefetch, FakeRelease};
use super::*;
use crate::cache::SystemClock;
use crate::memory::{Ledger, Step};
use crate::pool::Directive;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::runtime::Handle;

struct Rig {
    pool: Arc<FakePool>,
    prefetch: Arc<FakePrefetch>,
    directives: Arc<FakeDirectives>,
    release: Arc<FakeRelease>,
    signals: Signals,
}

fn rig() -> Rig {
    let pool: Arc<FakePool> = shared();
    let prefetch: Arc<FakePrefetch> = shared();
    let directives: Arc<FakeDirectives> = shared();
    let release: Arc<FakeRelease> = shared();
    let signals = Signals::new(
        &Handle::current(),
        pool.clone(),
        prefetch.clone(),
        directives.clone(),
        release.clone(),
        Ledger::new(1 << 30),
        Arc::new(SystemClock),
    );
    Rig { pool, prefetch, directives, release, signals }
}

fn events(rig: &Rig) -> Vec<String> {
    rig.pool.events.lock().unwrap().clone()
}

#[tokio::test]
async fn memory_warning_lowers_the_step_and_releases_slots_by_level() {
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
async fn pressure_and_background_stop_prefetch_drop_the_platform_memory_caches_and_forward_the_slot_directives() {
    for trigger in [|signals: &Signals| signals.memory_warning(Pressure::Critical), |signals: &Signals| signals.background()] {
        let rig = rig();
        *rig.pool.to_return.lock().unwrap() = vec![Directive::StopPrefetch, unload(1), Directive::DropMemoryCache];

        trigger(&rig.signals);

        eventually(|| !rig.directives.applied.lock().unwrap().is_empty() && rig.release.drops.load(Ordering::SeqCst) == 1).await;
        assert_eq!(rig.prefetch.stops.load(Ordering::SeqCst), 1);
        assert_eq!(*rig.directives.applied.lock().unwrap(), vec![vec![unload(1)]]);
    }
}

#[tokio::test]
async fn foreground_after_pressure_returns_the_step_to_normal() {
    let rig = rig();

    rig.signals.memory_warning(Pressure::Critical);
    rig.signals.foreground();

    eventually(|| events(&rig).contains(&"foreground".to_string())).await;
    assert_eq!(rig.pool.steps.lock().unwrap().last(), Some(&Step::Normal));
}

#[tokio::test]
async fn signals_apply_in_the_order_they_arrive() {
    let rig = rig();

    rig.signals.background();
    rig.signals.foreground();
    rig.signals.memory_warning(Pressure::Moderate);

    eventually(|| events(&rig).len() == 3).await;
    assert_eq!(events(&rig), vec!["background", "foreground", "release_under Moderate"]);
}

#[tokio::test]
async fn an_overflowing_queue_keeps_the_worst_pressure_and_the_last_lifecycle_signal() {
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
