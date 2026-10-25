use super::test_support::*;
use super::*;
use std::collections::{HashMap, HashSet};

struct Model {
    loaded: HashMap<usize, String>,
    playing: HashSet<usize>,
    created: HashSet<usize>,
    slots: usize,
}

impl Model {
    fn apply(&mut self, commands: &[Command]) {
        for command in commands {
            let slot = slot_of(command);
            assert!(slot < self.slots);
            match command {
                Command::Load { key, .. } => {
                    assert!(!self.loaded.contains_key(&slot), "load into a loaded slot {slot}");
                    self.loaded.insert(slot, key.clone());
                    self.created.insert(slot);
                }
                Command::Unload { .. } => {
                    assert!(self.loaded.remove(&slot).is_some(), "double unload {slot}");
                    self.playing.remove(&slot);
                }
                other => {
                    assert!(self.loaded.contains_key(&slot), "{other:?} to an unloaded slot");
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
            assert!(self.playing.len() <= 1, "two players after {command:?}");
        }
    }

    fn apply_directives(&mut self, directives: &[Directive]) {
        self.apply(&slot_commands(directives));
    }
}

#[test]
fn inv_pool_11_feed_chat_and_intro_share_one_pool_within_its_slots() {
    let mut pool = ring(5, 1);
    let mut model = Model { loaded: HashMap::new(), playing: HashSet::new(), created: HashSet::new(), slots: 5 };
    let feed = keys(50);
    model.apply(&pool.pin(0, "intro"));
    for (step, active) in walk(50, 600).into_iter().enumerate() {
        let now = step as u64 * 10;
        model.apply(&pool.plan(now, active, Some(active), &feed));
        model.apply(&pool.touch(now, &format!("chat{}", step % 4)));
        assert!(model.loaded.len() <= 5);
    }
    assert!(model.loaded.values().any(|key| key == "intro"));
    assert!(model.loaded.values().any(|key| key.starts_with("chat")));
    assert_eq!(model.created.len(), 5);
}

#[test]
fn inv_pool_11_kinds_never_play_together() {
    let mut pool = ring(5, 1);
    let mut model = Model { loaded: HashMap::new(), playing: HashSet::new(), created: HashSet::new(), slots: 5 };
    model.apply(&pool.pin(0, "intro"));
    model.apply(&pool.set_playing("intro", true));
    model.apply(&pool.touch(1, "chat"));
    model.apply(&pool.set_playing("chat", true));
    model.apply(&pool.plan(2, 5, Some(5), &keys(20)));
    assert_eq!(model.playing.len(), 1);
    model.apply(&pool.set_playing("intro", true));
    assert_eq!(model.playing.len(), 1);
}

#[test]
fn inv_pool_11_pool_policy_is_chosen_by_the_binding_owner_not_the_caller() {
    let mut pool = ring(4, 0);
    pool.plan(0, 3, Some(3), &keys(10));
    pool.touch(1, "chat");
    pool.pin(2, "intro");
    let owners: Vec<_> = pool.states().iter().map(|state| (state.is_pinned, state.retention)).collect();
    assert_eq!(owners.iter().filter(|(pinned, _)| *pinned).count(), 1);
    assert_eq!(owners.iter().filter(|(_, level)| *level != Retention::Cold).count(), 3);
}

#[test]
fn inv_pool_11_properties_hold_across_a_mixed_operation_walk() {
    let mut pool = ring_with(4, 1, |config| {
        config.trim_delay_ms = 200;
        config.hold_off_ms = 50;
    });
    let mut model = Model { loaded: HashMap::new(), playing: HashSet::new(), created: HashSet::new(), slots: 4 };
    let feed = keys(30);
    let mut dice = crate::seeded::Dice::new("inv_pool_11_properties_hold_across_a_mixed_operation_walk");
    for step in 0..3000u64 {
        let state = dice.next_u64();
        let now = step * 10;
        let pick = (state >> 33) % 14;
        let page = (state >> 40) as usize % 30;
        let key = format!("c{}", page % 5);
        match pick {
            0..=4 => model.apply(&pool.plan(now, page, Some(page), &feed)),
            5 => model.apply(&pool.touch(now, &key)),
            6 => model.apply(&pool.pin(now, &key)),
            7 => pool.unpin(&key),
            8 => model.apply(&pool.set_playing(&key, state & 1 == 0)),
            9 | 10 => model.apply(&pool.tick(now)),
            11 => model.apply_directives(&pool.release_under(now, if state & 2 == 0 { Pressure::Moderate } else { Pressure::Critical })),
            12 => {
                model.apply_directives(&pool.background());
                pool.foreground();
            }
            _ => model.apply(&pool.set_muted_all(state & 4 == 0)),
        }
        assert!(model.created.len() <= 4);
        assert_eq!(model.loaded.len(), pool.states().iter().filter(|s| s.retention != Retention::Cold).count());
    }
}

#[test]
fn inv_pool_05_replaying_the_same_calls_gives_the_same_commands() {
    let run = || {
        let mut pool = ring(4, 1);
        let mut all = Vec::new();
        for (step, active) in walk(30, 200).into_iter().enumerate() {
            all.extend(pool.plan(step as u64, active, Some(active), &keys(30)).iter().map(Shape::from));
            all.extend(pool.touch(step as u64, "chat").iter().map(Shape::from));
        }
        all
    };
    assert_eq!(run(), run());
}
