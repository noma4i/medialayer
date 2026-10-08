use super::test_support::*;
use super::*;

fn loads(commands: &[Command]) -> usize {
    commands.iter().filter(|command| matches!(command, Command::Load { .. })).count()
}

#[test]
fn hold_off_blocks_reload_of_the_same_window_until_it_passes() {
    let mut pool = ring_with(3, 1, |config| config.hold_off_ms = 1000);
    let list = keys(20);
    pool.plan(0, 5, Some(5), &list);
    pool.release_under(100, Pressure::Critical);
    assert!(pool.plan(200, 5, Some(5), &list).is_empty());
    assert!(pool.plan(1099, 5, Some(5), &list).is_empty());
    assert_eq!(loads(&pool.plan(1100, 5, Some(5), &list)), 3);
}

#[test]
fn hold_off_ends_when_the_window_changes() {
    let mut pool = ring_with(3, 1, |config| config.hold_off_ms = 1000);
    let list = keys(20);
    pool.plan(0, 5, Some(5), &list);
    pool.release_under(100, Pressure::Critical);
    assert_eq!(loads(&pool.plan(200, 9, Some(9), &list)), 3);
}

#[test]
fn pressure_without_unloads_sets_no_hold() {
    let mut pool = ring_with(3, 1, |config| config.hold_off_ms = 1000);
    pool.release_under(100, Pressure::Critical);
    assert_eq!(loads(&pool.plan(200, 5, Some(5), &keys(20))), 3);
}

#[test]
fn timer_demotes_hot_to_warm_then_warm_to_cold() {
    let mut pool = ring_with(3, 1, |config| config.trim_delay_ms = 1000);
    pool.plan(0, 5, Some(5), &keys(20));
    assert!(pool.tick(999).is_empty());
    assert_eq!(pool.tick(1000), [Command::Retain { slot: 0, level: Retention::Warm }, Command::Unload { slot: 1 }]);
    assert_eq!(pool.tick(1999), []);
    assert_eq!(pool.tick(2000), [Command::Unload { slot: 0 }]);
    assert!(pool.tick(10_000).is_empty());
}

#[test]
fn timer_release_does_not_reload_with_the_same_window() {
    let mut pool = ring_with(3, 1, |config| config.trim_delay_ms = 1000);
    let list = keys(20);
    pool.plan(0, 5, Some(5), &list);
    pool.tick(1000);
    pool.tick(2000);
    assert!(pool.plan(2100, 5, Some(5), &list).is_empty());
    assert_eq!(loads(&pool.plan(2200, 8, Some(8), &list)), 3);
}

#[test]
fn timer_trimmed_slot_is_retained_hot_when_it_starts_playing() {
    let mut pool = ring_with(3, 1, |config| config.trim_delay_ms = 1000);
    let list = keys(20);
    pool.plan(0, 5, Some(5), &list);
    pool.tick(1000);
    let commands = pool.plan(1100, 6, Some(6), &list);
    assert!(commands.contains(&Command::Retain { slot: 0, level: Retention::Hot }));
    assert!(commands.contains(&Command::Play { slot: 0 }));
}

#[test]
fn repeated_plan_after_a_timer_trim_is_empty() {
    let mut pool = ring_with(3, 1, |config| config.trim_delay_ms = 1000);
    let list = keys(20);
    pool.plan(0, 5, Some(5), &list);
    pool.tick(1000);
    assert!(pool.plan(1100, 5, Some(5), &list).is_empty());
}

#[test]
fn background_releases_all_but_the_playing_slot_after_stopping_prefetch() {
    let mut pool = ring(3, 1);
    pool.plan(0, 5, Some(5), &keys(20));
    assert_eq!(
        pool.background(),
        [Directive::StopPrefetch, Directive::Slot(Command::Unload { slot: 0 }), Directive::Slot(Command::Unload { slot: 1 })]
    );
    assert!(pool.background().is_empty());
    assert!(pool.plan(1, 5, Some(5), &keys(20)).is_empty());
    assert_eq!(pool.states().iter().filter(|state| state.is_playing).count(), 1);
}

#[test]
fn foreground_restores_one_slot_per_tick_nearest_first() {
    let mut pool = ring(3, 1);
    pool.plan(0, 5, Some(5), &keys(20));
    pool.background();
    pool.foreground();
    assert_eq!(pool.tick(10), [Command::Load { slot: 0, key: "k6".into() }]);
    assert_eq!(pool.tick(20), [Command::Load { slot: 1, key: "k4".into() }]);
    assert!(pool.tick(30).is_empty());
}

#[test]
fn foreground_without_background_restores_nothing() {
    let mut pool = ring(3, 1);
    pool.plan(0, 5, Some(5), &keys(20));
    pool.foreground();
    assert!(pool.tick(10).is_empty());
}

#[test]
fn release_unloads_one_key_and_release_all_unloads_everything() {
    let mut pool = ring(3, 1);
    pool.plan(0, 5, Some(5), &keys(20));
    assert_eq!(pool.release("k5"), [Command::Unload { slot: 2 }]);
    assert!(pool.release("k5").is_empty());
    assert_eq!(pool.release_all(), [Command::Unload { slot: 0 }, Command::Unload { slot: 1 }]);
    assert!(pool.release_all().is_empty());
}
