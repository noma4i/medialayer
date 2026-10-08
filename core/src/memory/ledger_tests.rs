use super::*;
use std::thread;

#[test]
fn reserve_counts_and_drop_releases() {
    let ledger = Ledger::new(1000);
    let first = ledger.reserve(Owner::Slot(0), 300).unwrap();
    let second = ledger.reserve(Owner::ProxyBuffer, 200).unwrap();
    assert_eq!((ledger.used(), ledger.ceiling()), (500, 1000));
    assert_eq!((first.bytes(), first.owner()), (300, Owner::Slot(0)));
    drop(first);
    assert_eq!(ledger.used(), 200);
    drop(second);
    assert_eq!(ledger.used(), 0);
}

#[test]
fn reserve_up_to_the_ceiling_is_admitted_and_one_byte_more_is_refused() {
    let ledger = Ledger::new(1000);
    let held = ledger.reserve(Owner::Queue, 1000).unwrap();
    let refused = ledger.reserve(Owner::Queue, 1).unwrap_err();
    assert_eq!(refused, Refused::Ceiling { owner: Owner::Queue, requested: 1, available: 0 });
    assert_eq!(ledger.used(), 1000);
    drop(held);
}

#[test]
fn a_refused_reserve_does_not_change_used() {
    let ledger = Ledger::new(100);
    let _held = ledger.reserve(Owner::ImageDecode, 60).unwrap();
    assert!(ledger.reserve(Owner::ImageDecode, 41).is_err());
    assert_eq!(ledger.used(), 60);
    assert_eq!(ledger.used_by(Owner::ImageDecode), 60);
}

#[test]
fn a_request_that_overflows_the_counter_is_refused_without_panic() {
    let ledger = Ledger::unbounded();
    let _held = ledger.reserve(Owner::Queue, 10).unwrap();
    assert!(ledger.reserve(Owner::Queue, u64::MAX).is_err());
    assert_eq!(ledger.used(), 10);
}

#[test]
fn used_by_sums_one_owner() {
    let ledger = Ledger::new(1000);
    let _a = ledger.reserve(Owner::Slot(0), 100).unwrap();
    let _b = ledger.reserve(Owner::Slot(1), 50).unwrap();
    let _c = ledger.reserve(Owner::Slot(0), 25).unwrap();
    assert_eq!((ledger.used_by(Owner::Slot(0)), ledger.used_by(Owner::Slot(1)), ledger.used_by(Owner::Queue)), (125, 50, 0));
}

#[test]
fn reserve_or_free_skips_freeing_when_it_fits() {
    let ledger = Ledger::new(100);
    let mut calls = 0;
    let mut freeing = |_needed: u64| {
        calls += 1;
        0
    };
    assert!(ledger.reserve_or_free(Owner::Queue, 50, &mut freeing).is_ok());
    assert_eq!(calls, 0);
}

#[test]
fn reserve_or_free_asks_for_the_missing_bytes_then_admits() {
    let ledger = Ledger::new(100);
    let mut victim = Some(ledger.reserve(Owner::Slot(0), 80).unwrap());
    let mut asked = Vec::new();
    let mut freeing = |needed: u64| {
        asked.push(needed);
        victim.take().map_or(0, |held| held.bytes())
    };
    let reservation = ledger.reserve_or_free(Owner::Slot(1), 60, &mut freeing).unwrap();
    assert_eq!(asked, vec![40]);
    assert_eq!((ledger.used(), reservation.bytes()), (60, 60));
}

#[test]
fn reserve_or_free_refuses_when_freeing_is_not_enough() {
    let ledger = Ledger::new(100);
    let _pinned = ledger.reserve(Owner::Slot(0), 90).unwrap();
    let mut freeing = |_needed: u64| 0;
    let refused = ledger.reserve_or_free(Owner::Slot(1), 50, &mut freeing).unwrap_err();
    assert_eq!(refused, Refused::Ceiling { owner: Owner::Slot(1), requested: 50, available: 10 });
    assert_eq!(ledger.used(), 90);
}

#[test]
fn a_pressure_step_lowers_admission_but_not_the_ceiling() {
    let ledger = Ledger::new(1000);
    ledger.set_pressure_step(Step::FewerSlots);
    assert_eq!(ledger.ceiling(), 1000);
    assert!(ledger.reserve(Owner::Queue, 900).is_err());
    let held = ledger.reserve(Owner::Queue, 800).unwrap();
    ledger.set_pressure_step(Step::Normal);
    assert!(ledger.reserve(Owner::Queue, 200).is_ok());
    drop(held);
}

#[test]
fn a_deeper_step_than_the_current_use_keeps_used_below_the_ceiling() {
    let ledger = Ledger::new(1000);
    let held = ledger.reserve(Owner::Queue, 900).unwrap();
    ledger.set_pressure_step(Step::PlaceholdersOnly);
    assert!(ledger.used() <= ledger.ceiling());
    assert!(ledger.reserve(Owner::Queue, 1).is_err());
    drop(held);
    assert_eq!(ledger.used(), 0);
}

#[test]
fn alloc_counts_the_buffer_and_releases_it_on_drop() {
    let ledger = Ledger::new(1000);
    let mut buffer = ledger.alloc(Owner::ProxyBuffer, 256).unwrap();
    assert!(buffer.vec_mut().capacity() >= 256);
    assert_eq!(ledger.used(), 256);
    drop(buffer);
    assert_eq!(ledger.used(), 0);
}

#[test]
fn alloc_that_the_allocator_cannot_serve_is_refused_without_panic() {
    let ledger = Ledger::unbounded();
    let refused = ledger.alloc(Owner::ProxyBuffer, u64::MAX / 2).err().unwrap();
    assert!(matches!(refused, Refused::Allocation { owner: Owner::ProxyBuffer, .. }));
    assert_eq!(ledger.used(), 0);
}

#[test]
fn concurrent_reserves_never_pass_the_ceiling_and_settle_to_zero() {
    let ledger = Ledger::new(10_000);
    let workers: Vec<_> = (0..8)
        .map(|index| {
            let ledger = ledger.clone();
            thread::spawn(move || {
                for round in 0..2_000u64 {
                    if let Ok(held) = ledger.reserve(Owner::Slot(index), 100 + round % 7) {
                        assert!(ledger.used() <= ledger.ceiling());
                        drop(held);
                    }
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(ledger.used(), 0);
}
