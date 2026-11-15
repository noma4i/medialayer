use super::*;
use std::thread;

#[test]
fn inv_memory_01_reserve_counts_and_drop_releases() {
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
fn inv_memory_01_reserve_up_to_the_ceiling_is_admitted_and_one_byte_more_is_refused() {
    let ledger = Ledger::new(1000);
    let held = ledger.reserve(Owner::Queue, 1000).unwrap();
    let refused = ledger.reserve(Owner::Queue, 1).unwrap_err();
    assert_eq!(refused, Refused::Ceiling { owner: Owner::Queue, requested: 1, available: 0 });
    assert_eq!(ledger.used(), 1000);
    drop(held);
}

#[test]
fn inv_memory_02_a_request_that_overflows_the_counter_is_refused_without_panic() {
    let ledger = Ledger::unbounded();
    let _held = ledger.reserve(Owner::Queue, 10).unwrap();
    assert!(ledger.reserve(Owner::Queue, u64::MAX).is_err());
    assert_eq!(ledger.used(), 10);
}

#[test]
fn inv_memory_01_used_by_sums_one_owner() {
    let ledger = Ledger::new(1000);
    let _a = ledger.reserve(Owner::Slot(0), 100).unwrap();
    let _b = ledger.reserve(Owner::Slot(1), 50).unwrap();
    let _c = ledger.reserve(Owner::Slot(0), 25).unwrap();
    assert_eq!((ledger.used_by(Owner::Slot(0)), ledger.used_by(Owner::Slot(1)), ledger.used_by(Owner::Queue)), (125, 50, 0));
}

#[test]
fn inv_memory_08_reserve_or_free_skips_freeing_when_it_fits() {
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
fn inv_memory_08_reserve_or_free_asks_for_the_missing_bytes_then_admits() {
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
fn inv_memory_08_reserve_or_free_refuses_when_freeing_is_not_enough() {
    let ledger = Ledger::new(100);
    let _pinned = ledger.reserve(Owner::Slot(0), 90).unwrap();
    let mut freeing = |_needed: u64| 0;
    let refused = ledger.reserve_or_free(Owner::Slot(1), 50, &mut freeing).unwrap_err();
    assert_eq!(refused, Refused::Ceiling { owner: Owner::Slot(1), requested: 50, available: 10 });
    assert_eq!(ledger.used(), 90);
}

#[test]
fn inv_memory_01_a_pressure_step_lowers_admission_but_not_the_ceiling() {
    let ledger = Ledger::new(1000);
    ledger.set_pressure_step(Step::FewerSlots);
    assert_eq!(ledger.ceiling(), 1000);
    assert!(ledger.reserve(Owner::Queue, 900).is_err());
    let held = ledger.reserve(Owner::Queue, 800).unwrap();
    ledger.set_pressure_step(Step::PlaceholdersOnly);
    assert!(ledger.reserve(Owner::Queue, 1).is_err());
    ledger.set_pressure_step(Step::Normal);
    assert!(ledger.reserve(Owner::Queue, 200).is_ok());
    drop(held);
}

#[test]
fn inv_memory_03_alloc_counts_the_buffer_and_releases_it_on_drop() {
    let ledger = Ledger::new(1000);
    let mut buffer = ledger.alloc(Owner::ProxyBuffer, 256).unwrap();
    assert_eq!(buffer.as_mut_slice().len(), 256);
    assert_eq!(ledger.used(), 256);
    drop(buffer);
    assert_eq!(ledger.used(), 0);
}

#[test]
fn inv_memory_02_alloc_that_the_allocator_cannot_serve_is_refused_without_panic() {
    let ledger = Ledger::unbounded();
    let refused = ledger.alloc(Owner::ProxyBuffer, u64::MAX / 2).err().unwrap();
    assert!(matches!(refused, Refused::Allocation { owner: Owner::ProxyBuffer, .. }));
    assert_eq!(ledger.used(), 0);
}

#[test]
fn inv_memory_01_concurrent_reserves_never_pass_the_ceiling_and_settle_to_zero() {
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

#[test]
fn inv_memory_01_set_ceiling_moves_the_admission_limit_and_keeps_existing_reservations() {
    let ledger = Ledger::new(100);
    let held = ledger.reserve(Owner::Queue, 60).unwrap();

    ledger.set_ceiling(50);

    assert_eq!(ledger.ceiling(), 50);
    assert_eq!(ledger.reserve(Owner::Queue, 1).unwrap_err(), Refused::Ceiling { owner: Owner::Queue, requested: 1, available: 0 });
    assert_eq!(ledger.used(), 60);
    ledger.set_ceiling(200);
    let extra = ledger.reserve(Owner::Queue, 100).unwrap();
    assert_eq!(ledger.used(), 160);
    drop((held, extra));
}

#[tokio::test]
async fn inv_memory_04_reserve_between_grants_what_is_free_up_to_the_wish_and_waits_below_the_floor() {
    let ledger = Ledger::new(100);
    let holder = ledger.reserve(Owner::Slot(0), 70).unwrap();

    let degraded = ledger.reserve_between(Owner::ProxyBuffer, 10, 50).await.unwrap();
    assert_eq!(degraded.bytes(), 30);
    drop(degraded);

    let waiter = tokio::spawn({
        let ledger = ledger.clone();
        async move { ledger.reserve_between(Owner::ProxyBuffer, 40, 50).await.map(|granted| granted.bytes()) }
    });
    crate::api::test_support::eventually(|| ledger.waiting() > 0).await;
    assert_eq!(ledger.used_by(Owner::ProxyBuffer), 0);

    drop(holder);

    assert_eq!(waiter.await.unwrap(), Ok(50));
    assert_eq!(ledger.waiting(), 0);
}

#[tokio::test]
async fn inv_memory_04_reserve_between_refuses_at_once_a_floor_that_can_never_fit() {
    let ledger = Ledger::new(100);

    let refused = ledger.reserve_between(Owner::ProxyBuffer, 101, 200).await.unwrap_err();

    assert_eq!(refused, Refused::Ceiling { owner: Owner::ProxyBuffer, requested: 101, available: 100 });
    assert_eq!(ledger.waiting(), 0);
}

#[tokio::test]
async fn inv_memory_14_a_cancelled_wait_leaves_no_waiter_and_no_bytes() {
    let ledger = Ledger::new(100);
    let holder = ledger.reserve(Owner::Slot(0), 70).unwrap();
    let waiter = tokio::spawn({
        let ledger = ledger.clone();
        async move { ledger.reserve_between(Owner::ImageDecode, 40, 40).await.map(|granted| granted.bytes()) }
    });
    crate::api::test_support::eventually(|| ledger.waiting() > 0).await;

    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());

    assert_eq!(ledger.waiting(), 0);
    assert_eq!((ledger.used(), ledger.used_by(Owner::ImageDecode)), (70, 0));
    drop(holder);
    let again = tokio::time::timeout(std::time::Duration::from_secs(5), ledger.reserve_between(Owner::ImageDecode, 100, 100)).await;
    assert_eq!(again.expect("the bytes stayed reserved").unwrap().bytes(), 100);
}

#[tokio::test]
async fn inv_memory_14_a_grant_the_caller_never_took_returns_its_bytes() {
    let ledger = Ledger::new(100);
    let holder = ledger.reserve(Owner::Slot(0), 70).unwrap();
    let task = tokio::spawn({
        let ledger = ledger.clone();
        async move { ledger.reserve_between(Owner::ImageDecode, 40, 40).await }
    });
    crate::api::test_support::eventually(|| ledger.waiting() > 0).await;
    drop(holder);
    crate::api::test_support::eventually(|| ledger.used_by(Owner::ImageDecode) != 0).await;
    assert_eq!(ledger.used_by(Owner::ImageDecode), 40, "the waiter was never granted");

    drop(task);
    crate::api::test_support::eventually(|| ledger.used_by(Owner::ImageDecode) == 0).await;

    assert_eq!((ledger.used(), ledger.used_by(Owner::ImageDecode), ledger.waiting()), (0, 0, 0));
}
