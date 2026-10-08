use super::test_support::*;
use super::*;

fn state(slot: usize, retention: Retention, playing: bool, distance: usize) -> SlotState {
    SlotState { slot, retention, is_playing: playing, is_pinned: false, distance }
}

fn unloaded(directives: &[Directive]) -> Vec<usize> {
    directives
        .iter()
        .filter_map(|directive| match directive {
            Directive::Slot(Command::Unload { slot }) => Some(*slot),
            _ => None,
        })
        .collect()
}

#[test]
fn moderate_unloads_only_non_playing_warm_slots_farthest_first() {
    let slots = [
        state(0, Retention::Hot, true, 0),
        state(1, Retention::Warm, false, 1),
        state(2, Retention::Warm, false, 3),
        state(3, Retention::Cold, false, 4),
        state(4, Retention::Hot, false, 1),
    ];
    assert_eq!(unloaded(&relieve(Pressure::Moderate, &slots)), [2, 1]);
}

#[test]
fn critical_orders_warm_then_hot_non_playing_then_playing_last() {
    let slots = [
        state(0, Retention::Hot, true, 0),
        state(1, Retention::Warm, false, 1),
        state(2, Retention::Warm, false, 3),
        state(3, Retention::Cold, false, 4),
        state(4, Retention::Hot, false, 1),
    ];
    assert_eq!(unloaded(&relieve(Pressure::Critical, &slots)), [2, 1, 4, 0]);
}

#[test]
fn cold_slots_get_no_commands() {
    let slots = [state(0, Retention::Cold, false, 0), state(1, Retention::Cold, false, 1)];
    assert_eq!(relieve(Pressure::Critical, &slots), [Directive::StopPrefetch, Directive::DropMemoryCache]);
    assert_eq!(relieve(Pressure::Moderate, &slots), [Directive::StopPrefetch]);
}

#[test]
fn ring_states_mark_playing_and_next_hot_others_warm_unbound_cold() {
    let mut ring = ring(4, 1);
    ring.plan(0, 5, Some(5), &keys(20));
    let states = ring.states();
    let by_slot = |slot: usize| states.iter().find(|state| state.slot == slot).unwrap();
    assert_eq!(by_slot(1).retention, Retention::Hot);
    assert!(by_slot(1).is_playing);
    assert_eq!(by_slot(2).retention, Retention::Hot);
    assert_eq!(by_slot(0).retention, Retention::Warm);
    assert_eq!(by_slot(3).retention, Retention::Cold);
    let stats = ring.stats();
    assert_eq!((stats.hot, stats.warm, stats.cold), (2, 1, 1));
}

#[test]
fn ring_release_under_critical_unbinds_so_slots_are_cold() {
    let mut pool = ring(3, 1);
    pool.plan(0, 5, Some(5), &keys(20));
    let directives = pool.release_under(1, Pressure::Critical);
    assert_eq!(unloaded(&directives).len(), 3);
    assert!(pool.states().iter().all(|state| state.retention == Retention::Cold));
}

#[test]
fn critical_orders_directives_stop_prefetch_then_slots_then_memory_cache_then_playing() {
    let slots = [
        state(0, Retention::Hot, true, 0),
        state(1, Retention::Warm, false, 1),
        state(4, Retention::Hot, false, 1),
    ];
    assert_eq!(
        relieve(Pressure::Critical, &slots),
        [
            Directive::StopPrefetch,
            Directive::Slot(Command::Unload { slot: 1 }),
            Directive::Slot(Command::Unload { slot: 4 }),
            Directive::DropMemoryCache,
            Directive::Slot(Command::Unload { slot: 0 }),
        ]
    );
}

#[test]
fn moderate_stops_prefetch_first_and_keeps_the_memory_cache() {
    let slots = [state(1, Retention::Warm, false, 1)];
    assert_eq!(
        relieve(Pressure::Moderate, &slots),
        [Directive::StopPrefetch, Directive::Slot(Command::Unload { slot: 1 })]
    );
}

#[test]
fn pinned_slots_survive_moderate_and_go_last_under_critical() {
    let pinned = |slot, retention, playing| SlotState { is_pinned: true, ..state(slot, retention, playing, 9) };
    let slots = [pinned(0, Retention::Warm, false), state(1, Retention::Warm, false, 1), pinned(2, Retention::Hot, true)];
    assert_eq!(unloaded(&relieve(Pressure::Moderate, &slots)), [1]);
    let critical = relieve(Pressure::Critical, &slots);
    assert_eq!(unloaded(&critical), [1, 0, 2]);
    assert_eq!(critical.last(), Some(&Directive::Slot(Command::Unload { slot: 2 })));
}
