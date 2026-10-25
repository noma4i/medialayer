use super::test_support::*;
use super::*;

fn playing_slots(pool: &Ring) -> Vec<usize> {
    pool.states().iter().filter(|state| state.is_playing).map(|state| state.slot).collect()
}

fn loaded(count: usize) -> Ring {
    let mut pool = ring(3, 1);
    pool.plan(0, 5, None, &keys(count));
    pool
}

#[test]
fn inv_pool_02_set_playing_plays_once_and_pauses_the_previous_player_first() {
    let mut pool = loaded(20);
    assert_eq!(pool.set_playing("k5", true), [Shape::Play { slot: 2 }]);
    assert!(pool.set_playing("k5", true).is_empty());
    assert_eq!(pool.set_playing("k6", true), [Shape::Pause { slot: 2 }, Shape::Play { slot: 0 }]);
    assert_eq!(playing_slots(&pool), [0]);
}

#[test]
fn inv_pool_02_set_playing_false_pauses_only_a_playing_key() {
    let mut pool = loaded(20);
    assert!(pool.set_playing("k5", false).is_empty());
    pool.set_playing("k5", true);
    assert_eq!(pool.set_playing("k5", false), [Shape::Pause { slot: 2 }]);
    assert!(pool.set_playing("k5", false).is_empty());
}

#[test]
fn inv_pool_02_set_playing_on_an_unbound_key_does_nothing() {
    let mut pool = loaded(20);
    pool.set_playing("k5", true);
    assert!(pool.set_playing("ghost", true).is_empty());
    assert_eq!(playing_slots(&pool), [2]);
}

#[test]
fn inv_pool_02_handoff_plays_the_target_before_pausing_the_source() {
    let mut pool = loaded(20);
    pool.set_playing("k5", true);
    assert_eq!(pool.handoff("k5", "k6"), [Shape::Play { slot: 0 }, Shape::Pause { slot: 2 }]);
    assert_eq!(playing_slots(&pool), [0]);
    assert!(pool.handoff("k5", "k6").is_empty());
}

#[test]
fn inv_pool_02_handoff_from_a_paused_or_unknown_source_changes_nothing() {
    let mut pool = loaded(20);
    assert!(pool.handoff("k5", "k6").is_empty());
    pool.set_playing("k5", true);
    assert!(pool.handoff("k5", "ghost").is_empty());
    assert!(pool.handoff("ghost", "k6").is_empty());
    assert_eq!(playing_slots(&pool), [2]);
}

#[test]
fn inv_pool_12_global_mute_changes_only_slots_whose_value_changes() {
    let mut pool = loaded(20);
    assert_eq!(pool.set_muted_all(true), [Shape::Mute { slot: 0 }, Shape::Mute { slot: 1 }, Shape::Mute { slot: 2 }]);
    assert!(pool.set_muted_all(true).is_empty());
    assert_eq!(pool.set_muted_all(false).len(), 3);
}

#[test]
fn inv_pool_12_slot_mute_survives_global_unmute() {
    let mut pool = loaded(20);
    assert_eq!(pool.set_muted("k5", true), [Shape::Mute { slot: 2 }]);
    assert!(pool.set_muted("k5", true).is_empty());
    assert_eq!(pool.set_muted_all(true), [Shape::Mute { slot: 0 }, Shape::Mute { slot: 1 }]);
    assert_eq!(pool.set_muted_all(false), [Shape::Unmute { slot: 0 }, Shape::Unmute { slot: 1 }]);
    assert_eq!(pool.set_muted("k5", false), [Shape::Unmute { slot: 2 }]);
}

#[test]
fn inv_pool_12_mute_set_before_load_applies_right_after_the_load() {
    let mut pool = ring(3, 0);
    assert!(pool.set_muted("a", true).is_empty());
    assert_eq!(pool.touch(1, "a"), [load(0, "a".into()), Shape::Mute { slot: 0 }]);
}

#[test]
fn inv_pool_12_slot_reused_for_an_unmuted_key_is_unmuted() {
    let mut pool = ring(2, 0);
    pool.set_muted("a", true);
    pool.touch(1, "a");
    pool.touch(2, "b");
    pool.set_playing("b", true);
    let commands = pool.touch(3, "c");
    assert_eq!(commands, [Shape::Unload { slot: 0 }, load(0, "c".into()), Shape::Unmute { slot: 0 }]);
}

#[test]
fn inv_pool_12_mute_flips_leave_playing_and_levels_untouched() {
    let mut pool = loaded(20);
    pool.set_playing("k5", true);
    let before = pool.states();
    for flip in 0..10 {
        pool.set_muted_all(flip % 2 == 0);
    }
    assert_eq!(pool.states(), before);
}
