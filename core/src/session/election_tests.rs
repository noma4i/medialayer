use super::*;
use std::collections::HashSet;

fn policy(group: &str, priority: u32, resume: Resume) -> Policy {
    Policy { group: group.into(), priority, resume }
}

fn play(lease: u64) -> Directive {
    Directive { lease, order: Order::Play }
}

fn pause(lease: u64) -> Directive {
    Directive { lease, order: Order::Pause }
}

fn idle(lease: u64) -> Directive {
    Directive { lease, order: Order::Idle }
}

fn with(contexts: &[(&str, Context)]) -> (Election, Vec<u64>) {
    let mut election = Election::new();
    let leases = contexts.iter().map(|(key, context)| election.register(key, context.policy())).collect();
    (election, leases)
}

#[test]
fn context_table_matches_priorities_groups_and_resume_modes() {
    let table = [
        (Context::Ring, "ring", 300, Resume::Automatic),
        (Context::Ambient, "ambient", 325, Resume::Automatic),
        (Context::Hero, "hero", 400, Resume::Manual),
        (Context::Overlay, "overlay", 450, Resume::Automatic),
        (Context::System, "system", 475, Resume::Automatic),
        (Context::Exclusive, "exclusive", 500, Resume::Automatic),
    ];
    for (context, group, priority, resume) in table {
        assert_eq!(context.policy(), policy(group, priority, resume));
    }
}

#[test]
fn eligible_only_in_foreground_with_play_intent() {
    let (mut election, leases) = with(&[("a", Context::Ring)]);
    assert!(!election.is_eligible(leases[0]));
    election.set_intent("a", leases[0], Intent::Play);
    assert!(election.is_eligible(leases[0]));
    election.set_foreground(false);
    assert!(!election.is_eligible(leases[0]));
}

#[test]
fn single_play_intent_gets_a_play_directive_once() {
    let (mut election, leases) = with(&[("a", Context::Ring)]);
    assert_eq!(election.set_intent("a", leases[0], Intent::Play), [play(leases[0])]);
    assert!(election.set_intent("a", leases[0], Intent::Play).is_empty());
    assert!(election.is_active("a", leases[0]));
    assert!(!election.is_active("other", leases[0]));
}

#[test]
fn higher_priority_wins_and_pauses_the_previous_winner() {
    let (mut election, l) = with(&[("ring", Context::Ring), ("sys", Context::System)]);
    election.set_intent("ring", l[0], Intent::Play);
    assert_eq!(election.set_intent("sys", l[1], Intent::Play), [pause(l[0]), play(l[1])]);
    assert_eq!(election.active(), Some(l[1]));
}

#[test]
fn lower_priority_never_displaces_a_higher_one() {
    let (mut election, l) = with(&[("sys", Context::System), ("ring", Context::Ring)]);
    election.set_intent("sys", l[0], Intent::Play);
    assert!(election.set_intent("ring", l[1], Intent::Play).is_empty());
    assert_eq!(election.active(), Some(l[0]));
}

#[test]
fn equal_priority_goes_to_the_latest_intent_then_key_then_lease() {
    let mut election = Election::new();
    let a = election.register("a", policy("g", 100, Resume::Automatic));
    let b = election.register("b", policy("g", 100, Resume::Automatic));
    election.set_intent("a", a, Intent::Play);
    assert_eq!(election.set_intent("b", b, Intent::Play), [pause(a), play(b)]);
}

#[test]
fn manual_loser_becomes_idle_and_stays_idle_after_the_winner_leaves() {
    let (mut election, l) = with(&[("chat", Context::Hero), ("edit", Context::Exclusive)]);
    election.set_intent("chat", l[0], Intent::Play);
    assert_eq!(election.set_intent("edit", l[1], Intent::Play), [pause(l[0]), idle(l[0]), play(l[1])]);
    assert_eq!(election.intent(l[0]), Some(Intent::Idle));
    assert_eq!(election.set_intent("edit", l[1], Intent::Idle), [pause(l[1])]);
    assert_eq!(election.active(), None);
}

#[test]
fn automatic_loser_keeps_intent_and_resumes_after_the_winner_leaves() {
    let (mut election, l) = with(&[("ring", Context::Ring), ("ambient", Context::Ambient)]);
    election.set_intent("ring", l[0], Intent::Play);
    election.set_intent("ambient", l[1], Intent::Play);
    assert_eq!(election.intent(l[0]), Some(Intent::Play));
    assert_eq!(election.unregister("ambient", l[1]), [pause(l[1]), play(l[0])]);
}

#[test]
fn unregister_with_the_wrong_key_or_lease_is_ignored() {
    let (mut election, l) = with(&[("a", Context::Ring)]);
    election.set_intent("a", l[0], Intent::Play);
    assert!(election.unregister("b", l[0]).is_empty());
    assert!(election.unregister("a", 99).is_empty());
    assert_eq!(election.active(), Some(l[0]));
}

#[test]
fn set_intent_with_the_wrong_key_or_lease_is_ignored() {
    let (mut election, l) = with(&[("a", Context::Ring)]);
    assert!(election.set_intent("b", l[0], Intent::Play).is_empty());
    assert!(election.set_intent("a", 99, Intent::Play).is_empty());
    assert_eq!(election.active(), None);
}

#[test]
fn suspension_blocks_ineligible_groups_and_thaw_resumes() {
    let (mut election, l) = with(&[("ring", Context::Ring)]);
    election.set_intent("ring", l[0], Intent::Play);
    assert_eq!(election.set_suspension(Reason::Interruption, true, None), Ok(vec![pause(l[0])]));
    assert_eq!(election.set_suspension(Reason::Interruption, true, None), Ok(vec![]));
    assert_eq!(election.set_suspension(Reason::Interruption, false, None), Ok(vec![play(l[0])]));
    assert_eq!(election.set_suspension(Reason::Interruption, false, None), Ok(vec![]));
}

#[test]
fn suspension_allow_groups_keep_listed_groups_playing() {
    let (mut election, l) = with(&[("ring", Context::Ring), ("sys", Context::System)]);
    election.set_intent("ring", l[0], Intent::Play);
    election.set_intent("sys", l[1], Intent::Play);
    let directives = election.set_suspension(Reason::AudioFocusTransient, true, Some(&["ring"])).unwrap();
    assert_eq!(directives, [pause(l[1]), play(l[0])]);
}

#[test]
fn suspension_of_a_manual_session_drops_its_intent() {
    let (mut election, l) = with(&[("chat", Context::Hero)]);
    election.set_intent("chat", l[0], Intent::Play);
    assert_eq!(election.set_suspension(Reason::Interruption, true, None), Ok(vec![pause(l[0]), idle(l[0])]));
    assert_eq!(election.set_suspension(Reason::Interruption, false, None), Ok(vec![]));
}

#[test]
fn background_pauses_without_dropping_manual_intent_and_foreground_resumes() {
    let (mut election, l) = with(&[("chat", Context::Hero)]);
    election.set_intent("chat", l[0], Intent::Play);
    assert_eq!(election.set_foreground(false), [pause(l[0])]);
    assert!(election.set_foreground(false).is_empty());
    assert_eq!(election.intent(l[0]), Some(Intent::Play));
    assert_eq!(election.set_foreground(true), [play(l[0])]);
}

#[test]
fn stop_all_idles_every_session_with_play_intent() {
    let (mut election, l) = with(&[("ring", Context::Ring), ("ambient", Context::Ambient), ("sys", Context::System)]);
    election.set_intent("ring", l[0], Intent::Play);
    election.set_intent("ambient", l[1], Intent::Play);
    assert_eq!(election.stop_all(), [pause(l[1]), idle(l[1]), idle(l[0])]);
    assert_eq!(election.intent(l[0]), Some(Intent::Idle));
    assert!(election.stop_all().is_empty());
}

#[test]
fn stop_manual_keeps_automatic_sessions() {
    let (mut election, l) = with(&[("ring", Context::Ring), ("chat", Context::Hero)]);
    election.set_intent("ring", l[0], Intent::Play);
    election.set_intent("chat", l[1], Intent::Play);
    assert_eq!(election.stop_manual(), [pause(l[1]), idle(l[1]), play(l[0])]);
    assert_eq!(election.intent(l[0]), Some(Intent::Play));
    assert!(election.stop_manual().is_empty());
}

#[test]
fn at_most_one_session_plays_across_a_random_walk() {
    let contexts = [Context::Ring, Context::Ambient, Context::Hero, Context::Overlay, Context::System, Context::Exclusive];
    let mut election = Election::new();
    let leases: Vec<u64> = contexts.iter().enumerate().map(|(i, context)| election.register(&format!("s{i}"), context.policy())).collect();
    let mut playing: HashSet<u64> = HashSet::new();
    let mut state = 7u64;
    for _ in 0..4000 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let index = (state >> 33) as usize % 6;
        let key = format!("s{index}");
        let directives = match (state >> 40) % 9 {
            0..=3 => election.set_intent(&key, leases[index], Intent::Play),
            4 | 5 => election.set_intent(&key, leases[index], Intent::Idle),
            6 => election.set_foreground(state & 8 == 0),
            7 => election.set_suspension(Reason::Interruption, state & 16 == 0, None).unwrap(),
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
fn applying_the_same_inputs_again_is_idempotent() {
    let (mut election, l) = with(&[("ring", Context::Ring), ("sys", Context::System)]);
    election.set_intent("ring", l[0], Intent::Play);
    election.set_intent("sys", l[1], Intent::Play);
    election.set_suspension(Reason::Manual, true, Some(&["sys"])).unwrap();
    for _ in 0..3 {
        assert!(election.set_intent("sys", l[1], Intent::Play).is_empty());
        assert!(election.set_foreground(true).is_empty());
        assert!(election.set_suspension(Reason::Manual, true, Some(&["sys"])).unwrap().is_empty());
    }
}

#[test]
fn unregistering_the_active_session_pauses_it_even_if_stale() {
    let (mut election, l) = with(&[("a", Context::Ring)]);
    election.set_intent("a", l[0], Intent::Play);
    assert_eq!(election.unregister("a", l[0]), [pause(l[0])]);
    assert_eq!(election.active(), None);
}

#[test]
fn volume_changes_never_reach_the_election() {
    let sources = [include_str!("election.rs"), include_str!("suspensions.rs"), include_str!("recovery.rs"), include_str!("watchdog_policy.rs")];
    assert!(sources.iter().all(|source| !source.to_lowercase().contains("mute")));
}

#[test]
fn a_hundred_pool_mute_flips_leave_the_election_untouched() {
    use crate::pool::{Command, PoolConfig, Ring};
    let (mut election, l) = with(&[("ring", Context::Ring)]);
    election.set_intent("ring", l[0], Intent::Play);
    let mut pool = Ring::new(PoolConfig::new(3, 1));
    pool.plan(0, 5, Some(5), &(0..20).map(|index| Some(format!("k{index}"))).collect::<Vec<_>>());
    let before = election.clone();
    let mut session_orders = 0;
    for flip in 0..100 {
        for command in pool.set_muted_all(flip % 2 == 0) {
            assert!(matches!(command, Command::Mute { .. } | Command::Unmute { .. }));
            session_orders += election.set_foreground(true).len();
        }
    }
    assert_eq!(session_orders, 0);
    assert_eq!(election.active(), before.active());
    assert!(election.is_active("ring", l[0]));
}
