use super::test_support::*;
use super::*;
use std::collections::{HashMap, HashSet};

fn page_of(key: &str) -> usize {
    key[1..].parse().unwrap()
}

#[test]
fn slot_wraps_page_by_slot_count() {
    let ring = ring(3, 1);
    assert_eq!((0..7).map(|page| ring.slot(page)).collect::<Vec<_>>(), [0, 1, 2, 0, 1, 2, 0]);
}

#[test]
fn pages_window_follows_reach_and_list_bounds() {
    let pool = ring(3, 1);
    assert_eq!(pool.pages(5, 20), 4..7);
    assert_eq!(pool.pages(0, 20), 0..2);
    assert_eq!(pool.pages(19, 20), 18..20);
    assert!(pool.pages(0, 0).is_empty());
    assert_eq!(ring(5, 2).pages(5, 20), 3..8);
}

#[test]
fn slots_are_clamped_to_hold_the_window() {
    assert_eq!(ring(2, 1).slots(), 3);
    assert_eq!(ring(2, 0).slots(), 2);
}

#[test]
fn first_plan_loads_window_plays_active_and_prerolls_neighbours() {
    let mut ring = ring(3, 1);
    let commands = ring.plan(0, 5, Some(5), &keys(20));
    assert_eq!(
        commands,
        [
            Command::Load { slot: 1, key: "k4".into() },
            Command::Load { slot: 2, key: "k5".into() },
            Command::Load { slot: 0, key: "k6".into() },
            Command::Play { slot: 2 },
            Command::Preroll { slot: 0 },
            Command::Preroll { slot: 1 },
        ]
    );
}

#[test]
fn scroll_reuses_the_freed_slot() {
    let mut ring = ring(3, 1);
    ring.plan(0, 5, Some(5), &keys(20));
    let commands = ring.plan(0, 6, Some(6), &keys(20));
    assert_eq!(
        commands,
        [
            Command::Unload { slot: 1 },
            Command::Load { slot: 1, key: "k7".into() },
            Command::Pause { slot: 2 },
            Command::Play { slot: 0 },
            Command::Preroll { slot: 1 },
            Command::Preroll { slot: 2 },
        ]
    );
}

#[test]
fn playback_outside_window_plays_nothing() {
    let mut ring = ring(3, 1);
    let commands = ring.plan(0, 5, Some(9), &keys(20));
    assert!(commands.iter().all(|command| !matches!(command, Command::Play { .. })));
}

#[test]
fn pages_without_source_get_no_load() {
    let mut ring = ring(3, 1);
    let mut list = keys(20);
    list[5] = None;
    let commands = ring.plan(0, 5, Some(5), &list);
    assert!(commands.iter().all(|command| !matches!(command, Command::Load { slot: 2, .. })));
    assert!(commands.iter().all(|command| !matches!(command, Command::Play { .. } | Command::Preroll { slot: 2 })));
}

#[test]
fn changed_key_for_same_page_reloads_the_slot() {
    let mut ring = ring(3, 1);
    ring.plan(0, 5, Some(5), &keys(20));
    let mut list = keys(20);
    list[6] = Some("other".into());
    let commands = ring.plan(0, 5, Some(5), &list);
    assert_eq!(commands[..2], [Command::Unload { slot: 0 }, Command::Load { slot: 0, key: "other".into() }]);
}

#[test]
fn p1_slots_stay_within_pool_over_1000_window_changes() {
    let mut ring = ring(3, 1);
    let list = keys(50);
    for active in walk(50, 1000) {
        for command in ring.plan(0, active, Some(active), &list) {
            let slot = match command {
                Command::Load { slot, .. } => slot,
                other => slot_of(&other),
            };
            assert!(slot < 3);
        }
    }
}

#[test]
fn p2_at_most_one_slot_plays_after_every_command() {
    let mut ring = ring(5, 2);
    let list = keys(50);
    let mut playing: HashSet<usize> = HashSet::new();
    for active in walk(50, 1000) {
        for command in ring.plan(0, active, Some(active), &list) {
            match command {
                Command::Play { slot } => {
                    playing.insert(slot);
                }
                Command::Pause { slot } | Command::Unload { slot } => {
                    playing.remove(&slot);
                }
                _ => {}
            }
            assert!(playing.len() <= 1);
        }
    }
}

#[test]
fn p3_unloaded_slot_gets_no_commands_until_load() {
    let mut ring = ring(3, 1);
    let list = keys(50);
    let mut unloaded: HashSet<usize> = HashSet::new();
    for active in walk(50, 1000) {
        for command in ring.plan(0, active, Some(active), &list) {
            match command {
                Command::Unload { slot } => {
                    assert!(!unloaded.contains(&slot));
                    unloaded.insert(slot);
                }
                Command::Load { slot, .. } => {
                    unloaded.remove(&slot);
                }
                other => {
                    assert!(!unloaded.contains(&slot_of(&other)));
                }
            }
        }
    }
}

#[test]
fn p4_preroll_hits_only_non_playing_window_pages_once_per_source() {
    let mut ring = ring(3, 1);
    let list = keys(50);
    let mut loaded: HashMap<usize, (String, usize)> = HashMap::new();
    let mut prerolled: HashSet<usize> = HashSet::new();
    let mut loads = 0;
    for active in walk(50, 1000) {
        let window = ring.pages(active, list.len());
        for command in ring.plan(0, active, Some(active), &list) {
            match command {
                Command::Load { slot, key } => {
                    loads += 1;
                    loaded.insert(slot, (key, loads));
                }
                Command::Unload { slot } => {
                    loaded.remove(&slot);
                }
                Command::Preroll { slot } => {
                    let (key, load_id) = loaded.get(&slot).unwrap();
                    assert!(window.contains(&page_of(key)));
                    assert_ne!(page_of(key), active);
                    assert!(prerolled.insert(*load_id));
                }
                _ => {}
            }
        }
    }
}

#[test]
fn p5_repeated_plan_with_same_window_is_empty() {
    let mut ring = ring(3, 1);
    let list = keys(50);
    for active in walk(50, 200) {
        ring.plan(0, active, Some(active), &list);
        assert!(ring.plan(0, active, Some(active), &list).is_empty());
    }
}

#[test]
fn key_of_names_the_key_a_slot_holds_until_it_is_unloaded() {
    let mut ring = ring(3, 1);
    ring.plan(0, 5, Some(5), &keys(20));

    assert_eq!((ring.key_of(2), ring.key_of(1)), (Some("k5"), Some("k4")));
    ring.release("k5");
    assert_eq!(ring.key_of(2), None);
}
