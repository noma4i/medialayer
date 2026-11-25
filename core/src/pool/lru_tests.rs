use super::test_support::*;
use super::*;

#[test]
fn inv_pool_15_claim_prefers_free_and_spare_then_farthest_unprotected_window_slot() {
    let mut pool = ring(3, 1);
    assert_eq!(pool.claim(1, "free"), [load(0, "free")]);
    assert!(pool.claim(2, "free").is_empty());
    pool.plan(3, 1, None, &keys(3));
    pool.set_playing("k0", true);
    pool.pin(4, "k2");
    let victim = pool.slot_of("k1").unwrap();
    assert_eq!(pool.claim(5, "outside"), [Shape::Unload { slot: victim }, load(victim, "outside")]);
    assert!(pool.slot_of("k0").is_some());
    assert!(pool.slot_of("k2").is_some());
    let commands = pool.plan(6, 1, None, &keys(3));
    assert!(commands.iter().any(|command| *command == Shape::Unload { slot: victim }));

    let mut pool = ring(4, 1);
    pool.plan(1, 1, None, &keys(3));
    pool.touch(2, "spare");
    let spare = pool.slot_of("spare").unwrap();
    assert_eq!(pool.claim(3, "outside"), [Shape::Unload { slot: spare }, load(spare, "outside")]);

    let mut pool = ring(3, 1);
    pool.plan(1, 1, None, &keys(3));
    pool.touch(2, "k0");
    let farthest = pool.slot_of("k2").unwrap();
    assert_eq!(pool.claim(3, "outside"), [Shape::Unload { slot: farthest }, load(farthest, "outside")]);
    pool.set_playing("outside", true);
    pool.plan(4, 1, None, &keys(3));
    assert_eq!(pool.slot_of("outside"), Some(farthest));
}

#[test]
fn inv_pool_15_touch_loads_unknown_keys_into_free_slots_and_ignores_known_ones() {
    let mut pool = ring(3, 0);
    assert_eq!(pool.touch(1, "a"), [load(0, "a")]);
    assert_eq!(pool.touch(2, "b"), [load(1, "b")]);
    assert!(pool.touch(3, "a").is_empty());
}

#[test]
fn inv_pool_15_full_pool_evicts_the_least_recently_used_key() {
    let mut pool = ring(3, 0);
    pool.touch(1, "a");
    pool.touch(2, "b");
    pool.touch(3, "c");
    pool.touch(4, "a");
    assert_eq!(pool.touch(5, "d"), [Shape::Unload { slot: 1 }, load(1, "d")]);
}

#[test]
fn inv_pool_15_playing_key_is_not_evicted() {
    let mut pool = ring(2, 0);
    pool.touch(1, "a");
    pool.touch(2, "b");
    pool.set_playing("a", true);
    assert_eq!(pool.touch(3, "c"), [Shape::Unload { slot: 1 }, load(1, "c")]);
}

#[test]
fn inv_pool_15_lru_never_evicts_ring_slots() {
    let mut pool = ring(4, 0);
    pool.plan(0, 5, Some(5), &keys(20));
    for (now, key) in ["a", "b", "c", "d", "e"].into_iter().enumerate() {
        pool.touch(now as u64 + 1, key);
    }
    assert!(pool.states().iter().any(|state| state.is_playing));
    assert_eq!(pool.states().iter().filter(|state| state.retention != Retention::Cold).count(), 4);
    assert!(pool.touch(9, "k5").is_empty());
}

#[test]
fn inv_pool_15_ring_window_reclaims_slots_held_by_lru_keys() {
    let mut pool = ring(3, 1);
    pool.touch(1, "a");
    pool.touch(2, "b");
    pool.touch(3, "c");
    let commands = pool.plan(4, 5, Some(5), &keys(20));
    assert_eq!(commands.iter().filter(|command| matches!(command, Command::Load { .. })).count(), 3);
    assert_eq!(commands.iter().filter(|command| matches!(command, Command::Unload { .. })).count(), 3);
}

#[test]
fn inv_pool_15_touch_in_background_loads_nothing() {
    let mut pool = ring(3, 0);
    pool.background();
    assert!(pool.touch(1, "a").is_empty());
}

#[test]
fn inv_pool_15_untouch_unloads_a_recently_used_key_but_keeps_window_and_pinned_keys() {
    let mut pool = ring(4, 1);
    pool.plan(0, 0, None, &keys(3));
    pool.pin(1, "intro");
    pool.touch(2, "chat");

    assert!(pool.untouch("k0").is_empty());
    assert!(pool.untouch("intro").is_empty());
    assert_eq!(pool.untouch("chat"), [Shape::Unload { slot: 3 }]);
}
