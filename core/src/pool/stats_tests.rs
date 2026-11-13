use super::test_support::*;
use super::*;

#[test]
fn inv_pool_13_hot_next_is_retained_without_playing() {
    let mut pool = ring_with(3, 1, |config| config.trim_delay_ms = 100);
    let list = keys(20);
    for page in 1..=11 {
        let commands = pool.plan(page as u64, page, Some(page), &list);
        let active = pool.slot_of(&format!("k{page}")).unwrap();
        let next = pool.slot_of(&format!("k{}", page + 1)).unwrap();
        let plays: Vec<_> = commands.iter().filter_map(|command| match command {
            Command::Play { slot } => Some(*slot),
            _ => None,
        }).collect();
        assert_eq!(plays, vec![active]);
        assert!(pool.states()[active].is_playing);
        assert!(!pool.states()[next].is_playing);
        assert_eq!(pool.states()[next].retention, Retention::Hot);
        assert_eq!((pool.stats().hot, pool.stats().playing), (2, 1));
        assert!(pool.plan(page as u64, page, Some(page), &list).is_empty());
    }
    pool.set_playing("k11", false);
    assert_eq!(pool.stats().playing, 0);
    pool.set_peeking("k11", true);
    assert_eq!(pool.stats().playing, 1);
    pool.release_all();
    assert_eq!(pool.stats().playing, 0);

    let mut pool = ring_with(5, 2, |config| config.trim_delay_ms = 100);
    pool.plan(0, 11, Some(11), &list);
    let next = pool.slot_of("k12").unwrap();
    assert!(pool.tick(100).iter().any(|command| *command == Shape::Retain { slot: next, level: Retention::Warm }));
    pool.plan(101, 10, None, &list);
    let commands = pool.plan(102, 11, Some(11), &list);
    assert!(commands.iter().any(|command| *command == Shape::Retain { slot: next, level: Retention::Hot }));
    assert!(!commands.iter().any(|command| *command == Shape::Play { slot: next }));
}

#[test]
fn inv_pool_13_stats_count_source_change_releases_and_reuses() {
    let mut pool = ring(3, 1);
    let list = keys(20);
    pool.plan(0, 5, Some(5), &list);
    assert_eq!(pool.stats().reuses, 0);
    pool.plan(1, 6, Some(6), &list);
    let stats = pool.stats();
    assert_eq!((stats.reuses, stats.released.source_change), (1, 1));
}

#[test]
fn inv_pool_13_stats_count_pressure_timer_and_background_reasons_separately() {
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
    assert_eq!((released.timer, released.pressure, released.background), (2, 1, 3));
}

#[test]
fn inv_pool_13_stats_are_stable_when_nothing_happens() {
    let mut pool = ring(3, 1);
    pool.plan(0, 5, Some(5), &keys(20));
    let before = pool.stats();
    pool.plan(1, 5, Some(5), &keys(20));
    pool.tick(2);
    assert_eq!(pool.stats(), before);
}
