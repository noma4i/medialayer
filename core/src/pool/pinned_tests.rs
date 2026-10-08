use super::test_support::*;
use super::*;

#[test]
fn inv_pool_07_pin_loads_the_key_and_is_idempotent() {
    let mut pool = ring(3, 1);
    assert_eq!(pool.pin(1, "intro"), [Command::Load { slot: 0, key: "intro".into() }]);
    assert!(pool.pin(2, "intro").is_empty());
}

#[test]
fn inv_pool_07_pinned_slot_survives_every_window_change() {
    let mut pool = ring(3, 1);
    pool.pin(1, "intro");
    let list = keys(50);
    for active in walk(50, 300) {
        for command in pool.plan(2, active, Some(active), &list) {
            assert_ne!(command, Command::Unload { slot: 0 });
        }
    }
    assert_ne!(pool.states()[0].retention, Retention::Cold);
}

#[test]
fn inv_pool_07_pinned_slot_survives_moderate_pressure_and_goes_last_under_critical() {
    let mut pool = ring(3, 1);
    pool.pin(1, "intro");
    pool.plan(2, 5, Some(5), &keys(20));
    let moderate = pool.release_under(3, Pressure::Moderate);
    assert!(!moderate.contains(&Directive::Slot(Command::Unload { slot: 0 })));
    let critical = pool.release_under(4, Pressure::Critical);
    assert_eq!(critical.last(), Some(&Directive::Slot(Command::Unload { slot: 0 })));
}

#[test]
fn inv_pool_07_lru_never_evicts_pinned_and_unpin_makes_it_evictable() {
    let mut pool = ring(2, 0);
    pool.pin(1, "intro");
    pool.touch(2, "a");
    assert_eq!(pool.touch(3, "b"), [Command::Unload { slot: 1 }, Command::Load { slot: 1, key: "b".into() }]);
    pool.unpin("intro");
    assert_eq!(pool.touch(4, "c"), [Command::Unload { slot: 0 }, Command::Load { slot: 0, key: "c".into() }]);
}

#[test]
fn inv_pool_07_pin_in_a_pool_full_of_pinned_slots_loads_nothing() {
    let mut pool = ring(2, 0);
    pool.pin(1, "a");
    pool.pin(2, "b");
    assert!(pool.pin(3, "c").is_empty());
}

#[test]
fn inv_pool_07_pinning_a_ring_key_keeps_it_when_it_leaves_the_window() {
    let mut pool = ring(3, 1);
    let list = keys(20);
    pool.plan(0, 5, Some(5), &list);
    assert!(pool.pin(1, "k5").is_empty());
    let commands = pool.plan(2, 12, Some(12), &list);
    assert!(!commands.contains(&Command::Unload { slot: 2 }));
}
