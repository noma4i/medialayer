use super::test_support::*;
use super::*;
use std::collections::{HashMap, HashSet};

fn page_of(key: &str) -> usize {
    key[1..].parse().unwrap()
}

#[test]
fn inv_pool_10_slot_wraps_page_by_slot_count() {
    let ring = ring(3, 1);
    assert_eq!((0..7).map(|page| ring.slot(page)).collect::<Vec<_>>(), [0, 1, 2, 0, 1, 2, 0]);
}

#[test]
fn inv_pool_10_pages_window_follows_reach_and_list_bounds() {
    let pool = ring(3, 1);
    assert_eq!(pool.pages(5, 20), 4..7);
    assert_eq!(pool.pages(0, 20), 0..2);
    assert_eq!(pool.pages(19, 20), 18..20);
    assert!(pool.pages(0, 0).is_empty());
    assert_eq!(ring(5, 2).pages(5, 20), 3..8);
}

#[test]
fn inv_pool_10_slots_are_clamped_to_hold_the_window() {
    assert_eq!(ring(2, 1).slots(), 3);
    assert_eq!(ring(2, 0).slots(), 2);
}

#[test]
fn inv_pool_10_first_plan_loads_window_plays_active_and_prerolls_neighbours() {
    let mut ring = ring(3, 1);
    let commands = ring.plan(0, 5, Some(5), &keys(20));
    assert_eq!(
        commands,
        [
            load(1, "k4".into()),
            load(2, "k5".into()),
            load(0, "k6".into()),
            Shape::Play { slot: 2 },
            Shape::Preroll { slot: 0 },
            Shape::Preroll { slot: 1 },
        ]
    );
}

#[test]
fn inv_pool_10_scroll_reuses_the_freed_slot() {
    let mut ring = ring(3, 1);
    ring.plan(0, 5, Some(5), &keys(20));
    let commands = ring.plan(0, 6, Some(6), &keys(20));
    assert_eq!(
        commands,
        [
            Shape::Unload { slot: 1 },
            load(1, "k7".into()),
            Shape::Pause { slot: 2 },
            Shape::Play { slot: 0 },
            Shape::Preroll { slot: 1 },
            Shape::Preroll { slot: 2 },
        ]
    );
}

#[test]
fn inv_pool_10_playback_outside_window_plays_nothing() {
    let mut ring = ring(3, 1);
    let commands = ring.plan(0, 5, Some(9), &keys(20));
    assert!(commands.iter().all(|command| !matches!(command, Command::Play { .. })));
}

#[test]
fn inv_pool_10_pages_without_source_get_no_load() {
    let mut ring = ring(3, 1);
    let mut list = keys(20);
    list[5] = None;
    let commands = ring.plan(0, 5, Some(5), &list);
    assert!(commands.iter().all(|command| !matches!(command, Command::Load { slot: 2, .. })));
    assert!(commands.iter().all(|command| !matches!(command, Command::Play { .. } | Command::Preroll { slot: 2 })));
}

#[test]
fn inv_pool_10_changed_key_for_same_page_reloads_the_slot() {
    let mut ring = ring(3, 1);
    ring.plan(0, 5, Some(5), &keys(20));
    let mut list = keys(20);
    list[6] = Some("other".into());
    let commands = ring.plan(0, 5, Some(5), &list);
    assert_eq!(commands[..2], [Shape::Unload { slot: 0 }, load(0, "other".into())]);
}

#[test]
fn inv_pool_01_slots_stay_within_pool_over_1000_window_changes() {
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
fn inv_pool_02_at_most_one_slot_plays_after_every_command() {
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
fn inv_pool_03_unloaded_slot_gets_no_commands_until_load() {
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
fn inv_pool_04_preroll_hits_only_non_playing_window_pages_once_per_source() {
    let mut ring = ring(3, 1);
    let list = keys(50);
    let mut loaded: HashMap<usize, (String, usize)> = HashMap::new();
    let mut prerolled: HashSet<usize> = HashSet::new();
    let mut loads = 0;
    for active in walk(50, 1000) {
        let window = ring.pages(active, list.len());
        for command in ring.plan(0, active, Some(active), &list) {
            match command {
                Command::Load { slot, key, .. } => {
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
fn inv_pool_05_repeated_plan_with_same_window_is_empty() {
    let mut ring = ring(3, 1);
    let list = keys(50);
    for active in walk(50, 200) {
        ring.plan(0, active, Some(active), &list);
        assert!(ring.plan(0, active, Some(active), &list).is_empty());
    }
}

#[test]
fn inv_pool_01_key_of_names_the_key_a_slot_holds_until_it_is_unloaded() {
    let mut ring = ring(3, 1);
    ring.plan(0, 5, Some(5), &keys(20));

    assert_eq!((ring.key_of(2), ring.key_of(1)), (Some("k5"), Some("k4")));
    ring.release("k5");
    assert_eq!(ring.key_of(2), None);
}

#[test]
fn inv_pool_10_growing_the_ring_adds_free_slots_and_shrinking_it_unloads_the_extra_ones() {
    let mut ring = ring(5, 1);
    ring.plan(0, 2, None, &keys(20));

    assert!(ring.set_slots(7).is_empty());
    assert_eq!(ring.slots(), 7);
    let commands = ring.set_slots(3);

    assert_eq!(commands, [Shape::Unload { slot: 3 }, Shape::Dispose { slot: 3 }, Shape::Dispose { slot: 4 }, Shape::Dispose { slot: 5 }, Shape::Dispose { slot: 6 }]);
    assert_eq!(ring.slots(), 3);
    assert_eq!(ring.stats().released.explicit, 1);
    assert!(ring.plan(1, 3, None, &keys(20)).iter().all(|command| slot_of(command) < 3));
}

#[test]
fn inv_pool_10_the_ring_never_shrinks_below_the_window_it_must_hold() {
    let mut ring = ring(5, 1);

    ring.set_slots(1);

    assert_eq!(ring.slots(), 3);
}

#[test]
fn inv_readahead_08_a_bound_slot_is_only_ever_hot_or_warm() {
    let mut pool = ring(5, 1);
    let pages = keys(9);
    let mut seen = Vec::new();
    for (step, page) in [0usize, 4, 1, 6, 3, 8].into_iter().enumerate() {
        let step = step as u64;
        pool.plan(step * 40_000, page, Some(page), &pages);
        for moment in [step * 40_000, step * 40_000 + 35_000] {
            for slot in 0..5 {
                if let Some(level) = pool.retention_of(slot) {
                    if !seen.contains(&level) {
                        seen.push(level);
                    }
                }
            }
            pool.tick(moment + 35_000);
        }
    }
    assert!(seen.contains(&Retention::Hot) && seen.contains(&Retention::Warm) && seen.len() == 2, "{seen:?}");
}
