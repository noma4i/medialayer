use super::*;

fn state(slot: usize, retention: Retention, playing: bool, distance: usize) -> SlotState {
    SlotState { slot, retention, is_playing: playing, distance }
}

fn unloaded(commands: &[Command]) -> Vec<usize> {
    commands
        .iter()
        .map(|command| match command {
            Command::Unload { slot } => *slot,
            other => panic!("unexpected {other:?}"),
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
    assert!(relieve(Pressure::Critical, &slots).is_empty());
    assert!(relieve(Pressure::Moderate, &slots).is_empty());
}

#[test]
fn ring_states_mark_playing_and_next_hot_others_warm_unbound_cold() {
    let mut ring = Ring::new(4, 1);
    ring.plan(5, Some(5), &(0..20).map(|index| Some(format!("k{index}"))).collect::<Vec<_>>());
    let states = ring.states(5);
    let by_slot = |slot: usize| states.iter().find(|state| state.slot == slot).unwrap();
    assert_eq!(by_slot(1).retention, Retention::Hot);
    assert!(by_slot(1).is_playing);
    assert_eq!(by_slot(2).retention, Retention::Hot);
    assert_eq!(by_slot(0).retention, Retention::Warm);
    assert_eq!(by_slot(3).retention, Retention::Cold);
}

#[test]
fn ring_release_under_critical_unbinds_so_slots_are_cold() {
    let mut ring = Ring::new(3, 1);
    let list: Vec<Option<String>> = (0..20).map(|index| Some(format!("k{index}"))).collect();
    ring.plan(5, Some(5), &list);
    let commands = ring.release_under(Pressure::Critical, 5);
    assert_eq!(commands.len(), 3);
    assert!(ring.states(5).iter().all(|state| state.retention == Retention::Cold));
}
