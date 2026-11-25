use super::test_support::*;
use super::*;
use crate::memory::{Ledger, Owner as MemoryOwner, Step};
use crate::seeded::Dice;
use std::collections::{HashMap, HashSet};

const STEPS: [Step; 6] = [Step::Normal, Step::LowerQuality, Step::ShorterBuffer, Step::FewerSlots, Step::PlayingSlotOnly, Step::PlaceholdersOnly];
const FEED: usize = 14;
const SLOT_BYTES: u64 = 24 * 1024 * 1024;
const OPERATIONS: u64 = 4_000;

struct Shadow {
    loaded: HashMap<usize, String>,
    playing: HashSet<usize>,
    created: HashSet<usize>,
    widest: usize,
}

impl Shadow {
    fn apply(&mut self, commands: &[Command]) {
        for command in commands {
            let slot = slot_of(command);
            assert!(slot < self.widest, "{command:?} addresses slot {slot} of at most {}", self.widest);
            match command {
                Command::Load { key, .. } => {
                    assert!(!self.loaded.contains_key(&slot), "{command:?} into a loaded slot");
                    assert!(!self.loaded.values().any(|loaded| loaded == key), "{command:?} loads a key that another slot holds");
                    self.loaded.insert(slot, key.clone());
                    self.created.insert(slot);
                }
                Command::Unload { .. } => {
                    assert!(self.loaded.remove(&slot).is_some(), "{command:?} of a slot that holds nothing");
                    self.playing.remove(&slot);
                }
                Command::Dispose { .. } => {
                    assert!(!self.loaded.contains_key(&slot), "{command:?} of a slot that still holds a source");
                }
                other => {
                    assert!(self.loaded.contains_key(&slot), "{other:?} reached an unloaded slot");
                    match other {
                        Command::Play { .. } => {
                            self.playing.insert(slot);
                        }
                        Command::Pause { .. } => {
                            self.playing.remove(&slot);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn apply_directives(&mut self, directives: &[Directive]) {
        self.apply(&slot_commands(directives));
    }
}

fn pinned_loaded(pool: &Ring) -> HashSet<String> {
    pool.states().iter().filter(|state| state.is_pinned && state.retention != Retention::Cold).filter_map(|state| pool.key_of(state.slot).map(str::to_string)).collect()
}

fn check_books(pool: &Ring, ledger: &Ledger, shadow: &Shadow, context: &str) {
    let live = pool.states().iter().filter(|state| state.retention != Retention::Cold).count();
    assert_eq!(shadow.loaded.len(), live, "{context}: the platform holds {} players, the pool counts {live}", shadow.loaded.len());
    for (slot, key) in &shadow.loaded {
        assert_eq!(pool.key_of(*slot), Some(key.as_str()), "{context}: slot {slot} holds another key than the platform loaded");
        assert_eq!(ledger.used_by(MemoryOwner::Slot(*slot)), SLOT_BYTES, "{context}: slot {slot} reservation");
    }
    let reserved: u64 = (0..shadow.widest).map(|slot| ledger.used_by(MemoryOwner::Slot(slot))).sum();
    assert_eq!(ledger.used(), reserved, "{context}: the ledger holds bytes that no slot owns");
    assert_eq!(reserved, shadow.loaded.len() as u64 * SLOT_BYTES, "{context}: reserved bytes against loaded slots");
    assert!(ledger.used() <= ledger.ceiling(), "{context}: used passes the ceiling");
    assert!(shadow.playing.len() <= 1, "{context}: {} slots play", shadow.playing.len());
    assert!(shadow.created.len() <= shadow.widest, "{context}: more players were created than the pool ever had slots");
}

#[test]
fn inv_pool_01_inv_pool_02_inv_pool_03_inv_pool_06_random_operations_keep_one_player_per_key_and_the_books_balanced() {
    let mut dice = Dice::new("inv_pool_01_inv_pool_02_inv_pool_03_inv_pool_06_random_operations_keep_one_player_per_key_and_the_books_balanced");
    let ledger = Ledger::new(SLOT_BYTES * 7 / 2);
    let mut config = PoolConfig::new(5, 1);
    config.trim_delay_ms = 600;
    config.hold_off_ms = 120;
    let mut pool = Ring::with_ledger(config, ledger.clone());
    let mut shadow = Shadow { loaded: HashMap::new(), playing: HashSet::new(), created: HashSet::new(), widest: 5 };
    let feed = keys(FEED);
    let mut now = 0u64;
    let mut step = Step::Normal;
    let mut is_background = false;
    let mut counts = [0u64; 17];
    for number in 0..OPERATIONS {
        now += dice.below(400);
        let page = dice.index(FEED);
        let key = format!("c{}", dice.below(6));
        let pinned_before = pinned_loaded(&pool);
        let keeps_pins = step != Step::PlaceholdersOnly;
        let pick = dice.below(17);
        counts[pick as usize] += 1;
        let (name, mut released_pins) = match pick {
            0..=3 => {
                let commands = pool.plan(now, page, dice.percent(80).then_some(page), &feed);
                assert!(!is_background || commands.is_empty(), "a plan produced commands in the background");
                shadow.apply(&commands);
                ("plan", false)
            }
            4 => {
                shadow.apply(&pool.touch(now, &key));
                ("touch", false)
            }
            5 => {
                shadow.apply(&pool.untouch(&key));
                ("untouch", false)
            }
            6 => {
                shadow.apply(&pool.pin(now, &key));
                ("pin", false)
            }
            7 => {
                pool.unpin(&key);
                ("unpin", false)
            }
            8 => {
                shadow.apply(&pool.set_playing(&key, dice.percent(60)));
                ("set_playing", false)
            }
            9 => {
                let other = format!("c{}", dice.below(6));
                let commands = pool.handoff(&key, &other);
                let play = commands.iter().position(|command| matches!(command, Command::Play { .. }));
                let pause = commands.iter().position(|command| matches!(command, Command::Pause { .. }));
                if let (Some(play), Some(pause)) = (play, pause) {
                    assert!(play < pause, "a handoff paused the source before it played the target");
                }
                shadow.apply(&commands);
                ("handoff", false)
            }
            10 => {
                shadow.apply(&pool.tick(now));
                ("tick", false)
            }
            11 => {
                let critical = dice.percent(40);
                let directives = pool.release_under(now, if critical { Pressure::Critical } else { Pressure::Moderate });
                shadow.apply_directives(&directives);
                ("release_under", critical)
            }
            12 => {
                if dice.percent(50) {
                    let directives = pool.background();
                    shadow.apply_directives(&directives);
                    is_background = true;
                    ("background", true)
                } else {
                    pool.foreground();
                    is_background = false;
                    ("foreground", false)
                }
            }
            13 => {
                step = STEPS[dice.index(6)];
                pool.set_step(step);
                ("set_step", false)
            }
            14 => {
                if dice.percent(70) {
                    shadow.apply(&pool.set_muted_all(dice.percent(50)));
                    ("mute_all", false)
                } else if dice.percent(50) {
                    let slots = dice.between(3, 8) as usize;
                    shadow.widest = shadow.widest.max(slots);
                    let commands = pool.set_slots(slots);
                    shadow.apply(&commands);
                    assert_eq!(pool.slots(), slots.max(3), "set_slots");
                    ("set_slots", true)
                } else {
                    shadow.apply(&pool.release(&key));
                    ("release", false)
                }
            }
            15 => {
                if dice.percent(10) {
                    shadow.apply(&pool.release_all());
                    ("release_all", true)
                } else {
                    shadow.apply(&pool.set_muted(&key, dice.percent(50)));
                    ("mute_key", false)
                }
            }
            _ => {
                let commands = pool.claim(now, &key);
                assert!(commands.iter().all(|command| !matches!(command, Command::Unload { slot } if shadow.playing.contains(slot))));
                shadow.apply(&commands);
                if pool.slot_of(&key).is_some() {
                    assert_eq!(shadow.loaded.values().filter(|loaded| *loaded == &key).count(), 1);
                }
                ("claim", false)
            }
        };
        if name == "release" {
            released_pins = pinned_before.contains(&key);
        }
        if name == "set_step" && step == Step::PlaceholdersOnly {
            released_pins = true;
        }
        let context = format!("operation {number} ({name})");
        check_books(&pool, &ledger, &shadow, &context);
        if keeps_pins && !released_pins && name != "background" {
            let still_loaded = pinned_before.iter().all(|key| shadow.loaded.values().any(|loaded| loaded == key));
            assert!(still_loaded, "{context}: a pinned key lost its slot");
        }
    }
    let released = pool.stats().released;
    assert!(released.timer > 0 && released.pressure > 0 && released.background > 0 && released.source_change > 0 && released.explicit > 0, "a release path never ran: {released:?}");
    assert!(counts.iter().all(|count| *count > 50), "an operation almost never ran: {counts:?}");
    assert!(shadow.created.len() > 2, "the walk never loaded anything");
    shadow.apply(&pool.release_all());
    assert_eq!(ledger.used(), 0, "release_all left reservations");
}
