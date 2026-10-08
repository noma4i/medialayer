use super::test_support::*;
use super::*;

#[test]
fn stats_count_source_change_releases_and_reuses() {
    let mut pool = ring(3, 1);
    let list = keys(20);
    pool.plan(0, 5, Some(5), &list);
    assert_eq!(pool.stats().reuses, 0);
    pool.plan(1, 6, Some(6), &list);
    let stats = pool.stats();
    assert_eq!((stats.reuses, stats.released.source_change), (1, 1));
}

#[test]
fn stats_count_pressure_timer_and_background_reasons_separately() {
    let mut pool = ring_with(3, 1, |config| config.trim_delay_ms = 1000);
    pool.plan(0, 5, Some(5), &keys(20));
    pool.tick(1000);
    pool.tick(2000);
    assert_eq!(pool.stats().released, Released { timer: 2, ..Released::default() });
    pool.release_under(3000, Pressure::Critical);
    assert_eq!(pool.stats().released.pressure, 1);
    pool.plan(10_000, 9, Some(9), &keys(20));
    pool.background();
    let released = pool.stats().released;
    assert_eq!((released.timer, released.pressure, released.background), (2, 1, 2));
}

#[test]
fn stats_are_stable_when_nothing_happens() {
    let mut pool = ring(3, 1);
    pool.plan(0, 5, Some(5), &keys(20));
    let before = pool.stats();
    pool.plan(1, 5, Some(5), &keys(20));
    pool.tick(2);
    assert_eq!(pool.stats(), before);
}
