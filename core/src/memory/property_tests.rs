use super::*;
use crate::pool::Pressure;
use crate::seeded::Dice;

fn owner(roll: u64) -> Owner {
    match roll % 4 {
        0 => Owner::Slot((roll / 4 % 8) as usize),
        1 => Owner::ProxyBuffer,
        2 => Owner::ImageDecode,
        _ => Owner::Queue,
    }
}

const STEPS: [Step; 6] = [Step::Normal, Step::LowerQuality, Step::ShorterBuffer, Step::FewerSlots, Step::PlayingSlotOnly, Step::PlaceholdersOnly];

#[test]
fn inv_memory_01_used_never_passes_the_ceiling_over_100000_random_operations() {
    let ceiling = 10_000u64;
    let ledger = Ledger::new(ceiling);
    let mut degradation = Degradation::default();
    let mut rng = Dice::new("inv_memory_01_r11_used_never_passes_the_ceiling_over_100000_random_operations");
    let mut held: Vec<Reservation> = Vec::new();
    let mut refusals = 0u64;
    let mut frees = 0u64;
    for _ in 0..100_000 {
        let before = ledger.used();
        match rng.below(5) {
            0 | 1 => {
                let bytes = rng.below(ceiling / 3) + 1;
                match ledger.reserve(owner(rng.next_u64()), bytes) {
                    Ok(reservation) => held.push(reservation),
                    Err(_) => {
                        refusals += 1;
                        assert_eq!(ledger.used(), before);
                    }
                }
            }
            2 => {
                if !held.is_empty() {
                    let index = rng.below(held.len() as u64) as usize;
                    held.swap_remove(index);
                }
            }
            3 => {
                let pressure = match rng.below(3) {
                    0 => None,
                    1 => Some(Pressure::Moderate),
                    _ => Some(Pressure::Critical),
                };
                let step = degradation.for_pressure(ledger.used(), ceiling, pressure);
                ledger.set_pressure_step(step);
            }
            _ => {
                let bytes = rng.below(ceiling / 2) + 1;
                let mut freeing = |needed: u64| {
                    let mut freed = 0;
                    while freed < needed && !held.is_empty() {
                        freed += held.swap_remove(0).bytes();
                    }
                    frees += 1;
                    freed
                };
                if let Ok(reservation) = ledger.reserve_or_free(owner(rng.next_u64()), bytes, &mut freeing) {
                    held.push(reservation);
                }
            }
        }
        assert!(ledger.used() <= ledger.ceiling());
        assert_eq!(ledger.used(), held.iter().map(Reservation::bytes).sum::<u64>());
    }
    assert!(refusals > 0 && frees > 0, "operations never hit the ceiling: {refusals} {frees}");
    held.clear();
    assert_eq!(ledger.used(), 0);
}

#[test]
fn inv_memory_01_a_random_step_sequence_keeps_admission_below_the_step_share() {
    let ledger = Ledger::new(1000);
    let mut rng = Dice::new("inv_memory_01_a_random_step_sequence_keeps_admission_below_the_step_share");
    let mut held = Vec::new();
    for _ in 0..2_000 {
        let step = STEPS[rng.below(6) as usize];
        ledger.set_pressure_step(step);
        let bytes = rng.below(300) + 1;
        let used = ledger.used();
        let limit = 1000 * u64::from(StepPlan::for_step(step).admission_percent) / 100;
        match ledger.reserve(Owner::Queue, bytes) {
            Ok(reservation) => {
                assert!(used + bytes <= limit);
                held.push(reservation);
            }
            Err(_) => assert!(used + bytes > limit),
        }
        if held.len() > 6 {
            held.remove(0);
        }
    }
}

async fn contend(ledger: &Ledger, tasks: u64, dice: crate::seeded::Dice, moving: bool) -> (u64, u64) {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    let (granted, refused) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let stop = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (ledger, stop) = (ledger.clone(), stop.clone());
        tokio::spawn(async move {
            while !stop.load(Ordering::SeqCst) {
                if !moving {
                    assert!(ledger.used() <= ledger.ceiling(), "used passes the ceiling");
                }
                tokio::task::yield_now().await;
            }
        })
    };
    let controller = {
        let (ledger, stop, mut dice) = (ledger.clone(), stop.clone(), dice.fork(999));
        tokio::spawn(async move {
            while moving && !stop.load(Ordering::SeqCst) {
                ledger.set_ceiling(if dice.percent(50) { 300 } else { 4_000 });
                ledger.set_pressure_step(STEPS[dice.index(6)]);
                tokio::time::sleep(Duration::from_micros(dice.between(50, 400))).await;
            }
        })
    };
    let workers: Vec<_> = (0..tasks)
        .map(|index| {
            let (ledger, granted, refused) = (ledger.clone(), granted.clone(), refused.clone());
            let mut lane = dice.fork(index);
            tokio::spawn(async move {
                for _ in 0..40 {
                    let floor = lane.between(1, 500);
                    let wanted = floor + lane.below(600);
                    let wait = Duration::from_millis(lane.between(1, 20));
                    match tokio::time::timeout(wait, ledger.reserve_between(owner(lane.next_u64()), floor, wanted)).await {
                        Ok(Ok(held)) => {
                            assert!(held.bytes() >= floor && held.bytes() <= wanted, "granted {} for {floor}..{wanted}", held.bytes());
                            granted.fetch_add(1, Ordering::SeqCst);
                            for _ in 0..lane.below(3) {
                                tokio::task::yield_now().await;
                            }
                        }
                        Ok(Err(_)) => {
                            refused.fetch_add(1, Ordering::SeqCst);
                        }
                        Err(_) => {}
                    }
                }
            })
        })
        .collect();
    for worker in workers {
        tokio::time::timeout(Duration::from_secs(20), worker).await.expect("a reservation task hung").unwrap();
    }
    stop.store(true, Ordering::SeqCst);
    watcher.await.unwrap();
    controller.await.unwrap();
    (granted.load(Ordering::SeqCst), refused.load(Ordering::SeqCst))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inv_memory_01_inv_memory_04_waiting_reservations_never_pass_the_ceiling_and_all_settle_to_zero() {
    let dice = crate::seeded::Dice::new("inv_memory_01_inv_memory_04_waiting_reservations_never_pass_the_ceiling_and_all_settle_to_zero");
    let ledger = Ledger::new(1_500);
    let (granted, _) = contend(&ledger, 16, dice, false).await;
    assert!(granted > 100, "the stand barely granted anything: {granted}");
    assert_eq!((ledger.used(), ledger.waiting()), (0, 0), "reservations or waiters leaked");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inv_memory_04_a_ceiling_and_a_step_that_keep_moving_never_strand_a_waiter_or_leak_bytes() {
    let dice = crate::seeded::Dice::new("inv_memory_04_a_ceiling_and_a_step_that_keep_moving_never_strand_a_waiter_or_leak_bytes");
    let ledger = Ledger::new(4_000);
    let (granted, refused) = contend(&ledger, 16, dice, true).await;
    assert!(granted > 50 && refused > 0, "the stand never hit both outcomes: {granted} granted, {refused} refused");
    assert_eq!((ledger.used(), ledger.waiting()), (0, 0), "reservations or waiters leaked");
}
