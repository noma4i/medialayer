use super::test_support::*;
use super::*;
use crate::memory::{Ledger, Owner as MemoryOwner, Step};

const ESTIMATE: u64 = 100;

fn tight(ceiling: u64, slots: usize, reach: usize) -> (Ring, Ledger) {
    let ledger = Ledger::new(ceiling);
    let mut config = PoolConfig::new(slots, reach);
    config.slot_bytes_estimate = ESTIMATE;
    (Ring::with_ledger(config, ledger.clone()), ledger)
}

fn loaded_keys(commands: &[Command]) -> Vec<String> {
    commands.iter().filter_map(|command| if let Command::Load { key, .. } = command { Some(key.clone()) } else { None }).collect()
}

fn live(pool: &Ring) -> usize {
    pool.states().iter().filter(|state| state.retention != Retention::Cold).count()
}

#[test]
fn inv_pool_06_a_ring_without_a_ledger_is_never_refused() {
    let mut pool = ring(3, 1);
    let commands = pool.plan(0, 5, Some(5), &keys(20));
    assert_eq!(loaded_keys(&commands).len(), 3);
    assert_eq!(pool.stats().memory_bytes, 3 * 24 * 1024 * 1024);
}

#[test]
fn inv_pool_06_the_pool_holds_no_more_slots_than_the_ledger_admits() {
    let (mut pool, ledger) = tight(250, 5, 2);
    let commands = pool.plan(0, 4, Some(4), &keys(10));
    assert_eq!(loaded_keys(&commands).len(), 2);
    assert_eq!(ledger.used(), 200);
    assert_eq!(live(&pool), 2);
}

#[test]
fn inv_pool_06_the_nearest_pages_win_the_budget_forward_first() {
    let (mut pool, _ledger) = tight(250, 5, 2);
    let mut loaded = loaded_keys(&pool.plan(0, 4, Some(4), &keys(10)));
    loaded.sort();
    assert_eq!(loaded, ["k4", "k5"]);
}

#[test]
fn inv_pool_06_a_refused_page_stays_a_placeholder_and_loads_once_room_appears() {
    let (mut pool, ledger) = tight(250, 5, 2);
    let list = keys(10);
    let outside = ledger.reserve(MemoryOwner::ImageDecode, 150).unwrap();
    assert_eq!(loaded_keys(&pool.plan(0, 4, Some(4), &list)), ["k4"]);
    assert!(pool.plan(1, 4, Some(4), &list).is_empty());
    drop(outside);
    let mut later = loaded_keys(&pool.plan(2, 4, Some(4), &list));
    later.sort();
    assert_eq!(later, ["k5"]);
    assert_eq!(ledger.used(), 200);
}

#[test]
fn inv_pool_06_memory_frees_the_least_recently_used_slots_for_a_page_in_the_window() {
    let (mut pool, ledger) = tight(200, 5, 2);
    pool.touch(1, "x");
    pool.touch(2, "y");
    assert_eq!(ledger.used(), 200);
    let commands = pool.plan(3, 4, Some(4), &keys(10));
    assert!(commands.iter().any(|command| *command == Shape::Unload { slot: 0 }) && commands.iter().any(|command| *command == Shape::Unload { slot: 1 }));
    let mut loaded = loaded_keys(&commands);
    loaded.sort();
    assert_eq!(loaded, ["k4", "k5"]);
    assert_eq!(ledger.used(), 200);
}

#[test]
fn inv_pool_06_memory_frees_farther_ring_slots_for_a_nearer_page() {
    let (mut pool, ledger) = tight(200, 5, 2);
    let list = keys(20);
    pool.plan(0, 10, Some(10), &list);
    assert_eq!(live(&pool), 2);
    let commands = pool.plan(1, 12, Some(12), &list);
    assert_eq!(loaded_keys(&commands), ["k12"]);
    assert!(commands.iter().any(|command| *command == Shape::Unload { slot: 1 }));
    assert_eq!(ledger.used(), 200);
    let mut bound: Vec<usize> = pool.states().iter().filter(|state| state.retention != Retention::Cold).map(|state| state.distance).collect();
    bound.sort();
    assert_eq!(bound, [0, 2]);
}

#[test]
fn inv_pool_06_the_playing_slot_is_never_freed_for_memory() {
    let (mut pool, ledger) = tight(100, 3, 0);
    pool.plan(0, 0, Some(0), &keys(5));
    assert!(pool.touch(1, "z").is_empty());
    assert!(pool.pin(2, "p").is_empty());
    assert_eq!(ledger.used(), 100);
    assert!(pool.states().iter().any(|state| state.is_playing));
}

#[test]
fn inv_pool_06_a_pinned_slot_is_not_freed_for_a_page() {
    let (mut pool, ledger) = tight(100, 3, 0);
    pool.pin(0, "p");
    let commands = pool.plan(1, 0, Some(0), &keys(5));
    assert!(loaded_keys(&commands).is_empty());
    assert!(pool.states().iter().any(|state| state.is_pinned));
    assert_eq!(ledger.used(), 100);
}

#[test]
fn inv_pool_06_release_and_release_all_free_the_reservation() {
    let (mut pool, ledger) = tight(1000, 5, 2);
    pool.plan(0, 5, Some(5), &keys(20));
    assert_eq!((ledger.used(), pool.stats().memory_bytes), (500, 500));
    pool.release("k6");
    assert_eq!((ledger.used(), pool.stats().memory_bytes, ledger.used_by(MemoryOwner::Slot(1))), (400, 400, 0));
    assert_eq!(ledger.used_by(MemoryOwner::Slot(0)), 100);
    pool.release_all();
    assert_eq!((ledger.used(), pool.stats().memory_bytes), (0, 0));
}

#[test]
fn inv_pool_06_the_timer_and_pressure_free_the_reservation() {
    let ledger = Ledger::new(1000);
    let mut config = PoolConfig::new(3, 1);
    config.slot_bytes_estimate = ESTIMATE;
    config.trim_delay_ms = 1000;
    let mut pool = Ring::with_ledger(config, ledger.clone());
    pool.plan(0, 5, Some(5), &keys(20));
    pool.tick(1000);
    assert_eq!(ledger.used(), 200);
    pool.release_under(1500, Pressure::Critical);
    assert_eq!(ledger.used(), 0);
}

#[test]
fn inv_pool_06_background_restore_waits_for_memory_and_keeps_the_saved_slots() {
    let (mut pool, ledger) = tight(300, 3, 1);
    let list = keys(20);
    pool.plan(0, 5, Some(5), &list);
    pool.background();
    assert_eq!(ledger.used(), 100);
    let outside = ledger.reserve(MemoryOwner::Queue, 200).unwrap();
    pool.foreground();
    assert!(loaded_keys(&pool.tick(10)).is_empty());
    drop(outside);
    assert_eq!(loaded_keys(&pool.tick(20)).len(), 1);
    assert_eq!(loaded_keys(&pool.tick(30)).len(), 1);
    assert_eq!(ledger.used(), 300);
}

#[test]
fn inv_pool_06_release_counts_as_explicit_not_source_change() {
    let mut pool = ring(3, 1);
    pool.plan(0, 5, Some(5), &keys(20));
    pool.release("k5");
    pool.release_all();
    let released = pool.stats().released;
    assert_eq!((released.explicit, released.source_change), (3, 0));
}

#[test]
fn inv_pool_06_fewer_slots_shrinks_slots_and_reach() {
    let (mut pool, ledger) = tight(10_000, 6, 2);
    let list = keys(20);
    pool.plan(0, 5, Some(5), &list);
    assert_eq!(live(&pool), 5);
    pool.set_step(Step::FewerSlots);
    pool.plan(1, 5, Some(5), &list);
    assert_eq!(live(&pool), 3);
    assert_eq!(ledger.used(), 300);
    assert_eq!(pool.pages(5, 20), 4..7);
}

#[test]
fn inv_pool_06_playing_slot_only_keeps_just_the_playing_slot() {
    let (mut pool, ledger) = tight(10_000, 6, 2);
    let list = keys(20);
    pool.plan(0, 5, Some(5), &list);
    pool.set_step(Step::PlayingSlotOnly);
    let commands = pool.plan(1, 5, Some(5), &list);
    assert!(loaded_keys(&commands).is_empty());
    assert_eq!(live(&pool), 1);
    assert!(pool.states().iter().any(|state| state.is_playing));
    assert_eq!(ledger.used(), 100);
}

#[test]
fn inv_pool_06_placeholders_only_unloads_everything_and_loads_nothing_until_the_step_lifts() {
    let (mut pool, ledger) = tight(10_000, 6, 2);
    let list = keys(20);
    pool.plan(0, 5, Some(5), &list);
    pool.set_step(Step::PlaceholdersOnly);
    let commands = pool.plan(1, 5, Some(5), &list);
    assert!(loaded_keys(&commands).is_empty());
    assert_eq!((live(&pool), ledger.used()), (0, 0));
    assert!(loaded_keys(&pool.plan(2, 6, Some(6), &list)).is_empty());
    pool.set_step(Step::Normal);
    assert_eq!(loaded_keys(&pool.plan(3, 6, Some(6), &list)).len(), 5);
}

#[test]
fn inv_pool_06_a_step_also_lowers_the_ledger_admission() {
    let (mut pool, ledger) = tight(1000, 5, 2);
    pool.set_step(Step::FewerSlots);
    assert!(ledger.reserve(MemoryOwner::Queue, 900).is_err());
    pool.set_step(Step::Normal);
    assert!(ledger.reserve(MemoryOwner::Queue, 900).is_ok());
}

#[test]
fn inv_pool_17_a_load_command_carries_the_reservation_and_the_ring_release_waits_for_its_last_holder() {
    let (mut pool, ledger) = tight(1000, 5, 2);
    let carried: Vec<Command> = pool.plan(0, 5, Some(5), &keys(20)).into_iter().filter(|command| matches!(command, Command::Load { .. })).collect();
    assert_eq!(carried.len(), 5);

    pool.release_all();

    assert_eq!(ledger.used(), 500, "the ring gave the bytes back while the commands still held them");
    drop(carried);
    assert_eq!(ledger.used(), 0);
}

#[test]
fn inv_pool_20_a_refused_admission_counts_the_release_under_way_and_evicts_no_more() {
    let (mut pool, ledger) = tight(300, 3, 0);
    let held: Vec<Command> = ["x", "y", "z"].iter().enumerate().flat_map(|(index, key)| pool.touch(index as u64 + 1, key)).collect();
    assert_eq!(ledger.used(), 300);
    let list = keys(5);
    let unloads = |commands: &[Command]| commands.iter().filter(|command| matches!(command, Command::Unload { .. })).count();

    let first = pool.plan(10, 2, None, &list);
    let second = pool.plan(11, 2, None, &list);

    assert_eq!((unloads(&first), loaded_keys(&first).len()), (1, 0));
    assert_eq!(unloads(&second), 0, "the second pass evicted again for bytes already on their way");
    assert!(pool.has_refused());
    drop(held);
    let refill = pool.retry_refused(12, None);
    assert_eq!(loaded_keys(&refill), ["k2"]);
    assert_eq!(ledger.used(), 300);
}
