use super::*;
use crate::api::slots::RetainLevel;
use crate::api::source::PlaybackRoute;
use SlotCall::*;

fn load(key: &str) -> SlotCall {
    Load { key: key.to_string(), route: PlaybackRoute { url: key.to_string(), headers: Default::default(), generation: None, hls: false, looping: true } }
}

fn pending_after(calls: Vec<SlotCall>) -> Vec<SlotCall> {
    let queue = Queue::default();
    for call in calls {
        queue.push(call);
    }
    std::iter::from_fn(|| queue.pop()).collect()
}

#[test]
fn inv_api_01_a_new_source_drops_every_command_for_the_old_one_but_keeps_the_sound_state() {
    let cases = [
        ("load after load", vec![load("a"), Preroll, Play, load("b")], vec![load("b")]),
        ("load after retain and pause", vec![Retain(RetainLevel::Warm), Pause, load("b")], vec![load("b")]),
        ("load keeps the last mute", vec![Mute, Play, Unmute, Mute, load("b")], vec![Mute, load("b")]),
        ("unload after load", vec![load("a"), Play, Unload], vec![Unload]),
        ("a new source drops the view and the seek of the old one", vec![Attach { view_token: 1 }, Seek { position_ms: 5 }, load("b")], vec![load("b")]),
        ("a view and a seek stay for the current source", vec![Attach { view_token: 1 }, Detach, Seek { position_ms: 5 }], vec![Attach { view_token: 1 }, Detach, Seek { position_ms: 5 }]),
        ("a reload keeps its unload", vec![Unload, load("b")], vec![Unload, load("b")]),
        ("a second load after a reload keeps the unload", vec![Unload, load("a"), Play, load("b")], vec![Unload, load("b")]),
        ("commands for the current source stay in order", vec![Preroll, Play, Pause, Mute], vec![Preroll, Play, Pause, Mute]),
    ];

    for (name, calls, expected) in cases {
        assert_eq!(pending_after(calls), expected, "{name}");
    }
}

#[test]
fn inv_api_01_an_overflowing_queue_folds_to_the_last_state_of_each_kind() {
    let mut calls = vec![load("a")];
    for _ in 0..50 {
        calls.extend([Preroll, Play, Pause, Mute, Unmute, Retain(RetainLevel::Hot), Retain(RetainLevel::Warm)]);
    }
    calls.push(Play);
    let queue = Queue::default();
    let mut deepest = 0;
    for call in calls {
        queue.push(call);
        deepest = deepest.max(queue.len());
    }

    let folded: Vec<SlotCall> = std::iter::from_fn(|| queue.pop()).collect();

    assert!(deepest <= LANE_CAPACITY, "held {deepest}");
    assert_eq!(folded.iter().rfind(|call| matches!(call, Play | Pause)), Some(&Play));
    assert_eq!(folded.iter().rfind(|call| matches!(call, Mute | Unmute)), Some(&Unmute));
    assert_eq!(folded.iter().rfind(|call| matches!(call, Retain(_))), Some(&Retain(RetainLevel::Warm)));
    assert_eq!(folded.iter().filter(|call| matches!(call, Load { .. })).count(), 1);
}
