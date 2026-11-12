use super::*;
use std::collections::{BTreeSet, HashSet};

fn policy(group: &str, priority: u32, resume: ResumeMode) -> Policy {
    Policy { groups: BTreeSet::from([group.to_string()]), priority, resume }
}

fn play(lease: u64) -> LeaseOrder {
    LeaseOrder { lease, order: Order::Play }
}

fn pause(lease: u64) -> LeaseOrder {
    LeaseOrder { lease, order: Order::Pause }
}

fn idle(lease: u64) -> LeaseOrder {
    LeaseOrder { lease, order: Order::Idle }
}

fn with(contexts: &[(&str, Context)]) -> (Election, Vec<u64>) {
    let mut election = Election::new();
    let leases = contexts.iter().map(|(key, context)| election.register(key, context.policy())).collect();
    (election, leases)
}

#[test]
fn inv_session_03_eligible_only_in_foreground_with_play_intent() {
    let (mut election, leases) = with(&[("a", Context::Ring)]);
    assert!(!election.is_eligible(leases[0]));
    election.set_intent("a", leases[0], Intent::Play);
    assert!(election.is_eligible(leases[0]));
    election.set_foreground(false);
    assert!(!election.is_eligible(leases[0]));
}

#[test]
fn inv_session_02_single_play_intent_gets_a_play_directive_once() {
    let (mut election, leases) = with(&[("a", Context::Ring)]);
    assert_eq!(election.set_intent("a", leases[0], Intent::Play), [play(leases[0])]);
    assert!(election.set_intent("a", leases[0], Intent::Play).is_empty());
    assert!(election.is_active("a", leases[0]));
    assert!(!election.is_active("other", leases[0]));
}

#[test]
fn inv_session_01_higher_priority_wins_and_pauses_the_previous_winner() {
    let (mut election, l) = with(&[("ring", Context::Ring), ("sys", Context::System)]);
    election.set_intent("ring", l[0], Intent::Play);
    assert_eq!(election.set_intent("sys", l[1], Intent::Play), [pause(l[0]), play(l[1])]);
    assert_eq!(election.active(), Some(l[1]));
}

#[test]
fn inv_session_01_lower_priority_never_displaces_a_higher_one() {
    let (mut election, l) = with(&[("sys", Context::System), ("ring", Context::Ring)]);
    election.set_intent("sys", l[0], Intent::Play);
    assert!(election.set_intent("ring", l[1], Intent::Play).is_empty());
    assert_eq!(election.active(), Some(l[0]));
}

#[test]
fn inv_session_01_equal_priority_goes_to_the_latest_intent_then_key_then_lease() {
    let mut election = Election::new();
    let a = election.register("a", policy("g", 100, ResumeMode::Automatic));
    let b = election.register("b", policy("g", 100, ResumeMode::Automatic));
    election.set_intent("a", a, Intent::Play);
    assert_eq!(election.set_intent("b", b, Intent::Play), [pause(a), play(b)]);
}

#[test]
fn inv_session_04_manual_loser_becomes_idle_and_stays_idle_after_the_winner_leaves() {
    let (mut election, l) = with(&[("chat", Context::Hero), ("edit", Context::Exclusive)]);
    election.set_intent("chat", l[0], Intent::Play);
    assert_eq!(election.set_intent("edit", l[1], Intent::Play), [pause(l[0]), idle(l[0]), play(l[1])]);
    assert_eq!(election.intent(l[0]), Some(Intent::Idle));
    assert_eq!(election.set_intent("edit", l[1], Intent::Idle), [pause(l[1])]);
    assert_eq!(election.active(), None);
}

#[test]
fn inv_session_04_automatic_loser_keeps_intent_and_resumes_after_the_winner_leaves() {
    let (mut election, l) = with(&[("ring", Context::Ring), ("ambient", Context::Ambient)]);
    election.set_intent("ring", l[0], Intent::Play);
    election.set_intent("ambient", l[1], Intent::Play);
    assert_eq!(election.intent(l[0]), Some(Intent::Play));
    assert_eq!(election.unregister("ambient", l[1]), [pause(l[1]), play(l[0])]);
}

#[test]
fn inv_session_05_unregister_with_the_wrong_key_or_lease_is_ignored() {
    let (mut election, l) = with(&[("a", Context::Ring)]);
    election.set_intent("a", l[0], Intent::Play);
    assert!(election.unregister("b", l[0]).is_empty());
    assert!(election.unregister("a", 99).is_empty());
    assert_eq!(election.active(), Some(l[0]));
}

#[test]
fn inv_session_05_set_intent_with_the_wrong_key_or_lease_is_ignored() {
    let (mut election, l) = with(&[("a", Context::Ring)]);
    assert!(election.set_intent("b", l[0], Intent::Play).is_empty());
    assert!(election.set_intent("a", 99, Intent::Play).is_empty());
    assert_eq!(election.active(), None);
}

#[test]
fn inv_session_03_suspension_blocks_ineligible_groups_and_thaw_resumes() {
    let (mut election, l) = with(&[("ring", Context::Ring)]);
    election.set_intent("ring", l[0], Intent::Play);
    assert_eq!(election.set_suspension(1, true, None), Ok(vec![pause(l[0])]));
    assert_eq!(election.set_suspension(1, true, None), Ok(vec![]));
    assert_eq!(election.set_suspension(1, false, None), Ok(vec![play(l[0])]));
    assert_eq!(election.set_suspension(1, false, None), Ok(vec![]));
}

#[test]
fn inv_session_03_suspension_allow_groups_keep_listed_groups_playing() {
    let (mut election, l) = with(&[("ring", Context::Ring), ("sys", Context::System)]);
    election.set_intent("ring", l[0], Intent::Play);
    election.set_intent("sys", l[1], Intent::Play);
    let directives = election.set_suspension(2, true, Some(&["ring"])).unwrap();
    assert_eq!(directives, [pause(l[1]), play(l[0])]);
}

#[test]
fn inv_session_04_suspension_of_a_manual_session_drops_its_intent() {
    let (mut election, l) = with(&[("chat", Context::Hero)]);
    election.set_intent("chat", l[0], Intent::Play);
    assert_eq!(election.set_suspension(1, true, None), Ok(vec![pause(l[0]), idle(l[0])]));
    assert_eq!(election.set_suspension(1, false, None), Ok(vec![]));
}

#[test]
fn inv_session_03_background_pauses_without_dropping_manual_intent_and_foreground_resumes() {
    let (mut election, l) = with(&[("chat", Context::Hero)]);
    election.set_intent("chat", l[0], Intent::Play);
    assert_eq!(election.set_foreground(false), [pause(l[0])]);
    assert!(election.set_foreground(false).is_empty());
    assert_eq!(election.intent(l[0]), Some(Intent::Play));
    assert_eq!(election.set_foreground(true), [play(l[0])]);
}

#[test]
fn inv_session_04_stop_all_idles_every_session_with_play_intent() {
    let (mut election, l) = with(&[("ring", Context::Ring), ("ambient", Context::Ambient), ("sys", Context::System)]);
    election.set_intent("ring", l[0], Intent::Play);
    election.set_intent("ambient", l[1], Intent::Play);
    assert_eq!(election.stop_all(), [pause(l[1]), idle(l[1]), idle(l[0])]);
    assert_eq!(election.intent(l[0]), Some(Intent::Idle));
    assert!(election.stop_all().is_empty());
}

#[test]
fn inv_session_04_stop_manual_keeps_automatic_sessions() {
    let (mut election, l) = with(&[("ring", Context::Ring), ("chat", Context::Hero)]);
    election.set_intent("ring", l[0], Intent::Play);
    election.set_intent("chat", l[1], Intent::Play);
    assert_eq!(election.stop_manual(), [pause(l[1]), idle(l[1]), play(l[0])]);
    assert_eq!(election.intent(l[0]), Some(Intent::Play));
    assert!(election.stop_manual().is_empty());
}

#[test]
fn inv_session_01_at_most_one_session_plays_across_a_random_walk() {
    let contexts = [Context::Ring, Context::Ambient, Context::Hero, Context::Overlay, Context::System, Context::Exclusive];
    let mut election = Election::new();
    let leases: Vec<u64> = contexts.iter().enumerate().map(|(i, context)| election.register(&format!("s{i}"), context.policy())).collect();
    let mut playing: HashSet<u64> = HashSet::new();
    let mut dice = crate::seeded::Dice::new("inv_session_01_at_most_one_session_plays_across_a_random_walk");
    for _ in 0..4000 {
        let state = dice.next_u64();
        let index = (state >> 33) as usize % 6;
        let key = format!("s{index}");
        let directives = match (state >> 40) % 9 {
            0..=3 => election.set_intent(&key, leases[index], Intent::Play),
            4 | 5 => election.set_intent(&key, leases[index], Intent::Idle),
            6 => election.set_foreground(state & 8 == 0),
            7 => election.set_suspension(1, state & 16 == 0, None).unwrap(),
            _ => election.stop_manual(),
        };
        for directive in directives {
            match directive.order {
                Order::Play => {
                    playing.insert(directive.lease);
                }
                Order::Pause | Order::Idle => {
                    playing.remove(&directive.lease);
                }
            }
            assert!(playing.len() <= 1);
        }
        assert_eq!(playing.iter().copied().next(), election.active());
    }
}

#[test]
fn inv_session_02_applying_the_same_inputs_again_is_idempotent() {
    let (mut election, l) = with(&[("ring", Context::Ring), ("sys", Context::System)]);
    election.set_intent("ring", l[0], Intent::Play);
    election.set_intent("sys", l[1], Intent::Play);
    election.set_suspension(3, true, Some(&["sys"])).unwrap();
    for _ in 0..3 {
        assert!(election.set_intent("sys", l[1], Intent::Play).is_empty());
        assert!(election.set_foreground(true).is_empty());
        assert!(election.set_suspension(3, true, Some(&["sys"])).unwrap().is_empty());
    }
}

fn groups(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

#[test]
fn inv_session_14_a_group_change_re_elects_under_a_suspension() {
    let (mut election, l) = with(&[("a", Context::Ring)]);
    election.set_intent("a", l[0], Intent::Play);
    election.set_suspension(1, true, Some(&["chat"])).unwrap();
    assert_eq!(election.active(), None);
    assert_eq!(election.set_groups("a", l[0], groups(&["chat"])), [play(l[0])]);
    assert!(election.set_groups("a", l[0], groups(&["chat"])).is_empty());
    assert_eq!(election.set_groups("a", l[0], groups(&["ring"])), [pause(l[0])]);
    assert!(election.set_groups("other", l[0], groups(&["chat"])).is_empty());
}

#[test]
fn inv_session_14_a_key_plays_only_if_one_of_its_groups_is_allowed_by_every_suspension() {
    let (mut election, l) = with(&[("a", Context::Ring)]);
    election.set_intent("a", l[0], Intent::Play);
    election.set_groups("a", l[0], groups(&["ring", "system"]));
    assert_eq!(election.set_suspension(1, true, Some(&["system"])).unwrap(), []);
    assert_eq!(election.active(), Some(l[0]));
    assert_eq!(election.set_suspension(2, true, Some(&["ring"])).unwrap(), [pause(l[0])]);
    assert_eq!(election.set_suspension(2, false, None).unwrap(), [play(l[0])]);
}

#[test]
fn inv_session_05_unregistering_the_active_session_pauses_it_even_if_stale() {
    let (mut election, l) = with(&[("a", Context::Ring)]);
    election.set_intent("a", l[0], Intent::Play);
    assert_eq!(election.unregister("a", l[0]), [pause(l[0])]);
    assert_eq!(election.active(), None);
}
