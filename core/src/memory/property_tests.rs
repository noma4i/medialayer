use super::*;
use crate::pool::Pressure;

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

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
fn r11_used_never_passes_the_ceiling_over_100000_random_operations() {
    let ceiling = 10_000u64;
    let ledger = Ledger::new(ceiling);
    let mut degradation = Degradation::default();
    let mut rng = Lcg(42);
    let mut held: Vec<Reservation> = Vec::new();
    let mut refusals = 0u64;
    let mut frees = 0u64;
    for _ in 0..100_000 {
        let before = ledger.used();
        match rng.below(5) {
            0 | 1 => {
                let bytes = rng.below(ceiling / 3) + 1;
                match ledger.reserve(owner(rng.next()), bytes) {
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
                if let Ok(reservation) = ledger.reserve_or_free(owner(rng.next()), bytes, &mut freeing) {
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
fn a_random_step_sequence_keeps_admission_below_the_step_share() {
    let ledger = Ledger::new(1000);
    let mut rng = Lcg(7);
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
