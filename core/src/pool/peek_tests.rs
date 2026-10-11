use super::test_support::*;
use super::*;

fn loaded(count: usize) -> Ring {
    let mut pool = ring(3, 1);
    pool.plan(0, 5, None, &keys(count));
    pool
}

fn playing_slots(pool: &Ring) -> Vec<usize> {
    pool.states().iter().filter(|state| state.is_playing).map(|state| state.slot).collect()
}

#[test]
fn inv_pool_30_peeking_plays_the_neighbour_muted_and_leaves_the_playing_page_alone() {
    let mut pool = loaded(20);
    pool.set_playing("k5", true);
    assert_eq!(pool.set_peeking("k6", true), [Command::Mute { slot: 0 }, Command::Play { slot: 0 }]);
    assert_eq!(playing_slots(&pool), [2]);
    assert!(pool.set_peeking("k6", true).is_empty());
}

#[test]
fn inv_pool_30_promoting_a_peeked_page_keeps_its_player_running_and_unmutes_it() {
    let mut pool = loaded(20);
    pool.set_playing("k5", true);
    pool.set_peeking("k6", true);
    assert_eq!(pool.set_playing("k6", true), [Command::Pause { slot: 2 }, Command::Unmute { slot: 0 }]);
    assert_eq!(playing_slots(&pool), [0]);
}

#[test]
fn inv_pool_30_ending_a_peek_pauses_the_neighbour_and_restores_its_sound() {
    let mut pool = loaded(20);
    pool.set_peeking("k6", true);
    assert_eq!(pool.set_peeking("k6", false), [Command::Pause { slot: 0 }, Command::Unmute { slot: 0 }]);
    assert!(pool.set_peeking("k6", false).is_empty());
}

#[test]
fn inv_pool_30_a_new_peek_stops_the_previous_one_and_a_playing_or_unbound_key_never_peeks() {
    let mut pool = loaded(20);
    pool.set_playing("k5", true);
    pool.set_peeking("k6", true);
    assert_eq!(pool.set_peeking("k4", true), [Command::Pause { slot: 0 }, Command::Unmute { slot: 0 }, Command::Mute { slot: 1 }, Command::Play { slot: 1 }]);
    assert!(pool.set_peeking("k5", true).is_empty());
    assert!(pool.set_peeking("ghost", true).is_empty());
}
