use super::*;
use crate::api::source::PlaybackRoute;
use SlotCall::*;

fn load(key: &str) -> SlotCall {
    Load { key: key.to_string(), route: PlaybackRoute { url: key.to_string(), headers: Default::default(), generation: None, hls: false, looping: true }, generation: 0 }
}

fn drain(queue: &Queue) -> Vec<SlotCall> {
    std::iter::from_fn(|| match queue.take()? {
        Next::Call { call, .. } => Some(call),
        Next::Dispose => None,
    })
    .collect()
}

fn pending_after(calls: Vec<SlotCall>) -> Vec<SlotCall> {
    let queue = Queue::default();
    for call in calls {
        queue.push(call, None);
    }
    drain(&queue)
}

#[test]
fn inv_api_01_inv_api_16_a_new_source_drops_every_command_for_the_old_one_but_keeps_the_sound_state() {
    let cases = [
        ("load after load", vec![load("a"), Preroll, Play, load("b")], vec![load("b")]),
        ("load after readahead and pause", vec![Readahead { ms: 8 }, Pause, load("b")], vec![load("b")]),
        ("load keeps the last mute", vec![Mute, Play, Unmute, Mute, load("b")], vec![Mute, load("b")]),
        ("unload after load", vec![load("a"), Play, Unload], vec![Unload]),
        ("a new source drops the view and the seek of the old one", vec![Attach { view_token: 1 }, Seek { position_ms: 5 }, load("b")], vec![load("b")]),
        ("a new source drops the rate of the old one", vec![Rate { rate: 2.0 }, load("b")], vec![load("b")]),
        ("a view and a seek stay for the current source", vec![Attach { view_token: 1 }, Detach, Seek { position_ms: 5 }], vec![Attach { view_token: 1 }, Detach, Seek { position_ms: 5 }]),
        ("a reload keeps its unload", vec![Unload, load("b")], vec![Unload, load("b")]),
        ("a second load after a reload keeps the unload", vec![Unload, load("a"), Play, load("b")], vec![Unload, load("b")]),
        ("a pause drops a play of the current source that has not run, the rest stays in order", vec![Preroll, Play, Pause, Mute], vec![Preroll, Pause, Mute]),
    ];

    for (name, calls, expected) in cases {
        assert_eq!(pending_after(calls), expected, "{name}");
    }
}

#[test]
fn inv_api_01_inv_api_16_an_overflowing_queue_folds_to_the_last_state_of_each_kind() {
    let mut calls = vec![load("a"), Rate { rate: 1.5 }, Rate { rate: 2.0 }];
    for _ in 0..50 {
        calls.extend([Preroll, Play, Pause, Mute, Unmute, Readahead { ms: 1 }, Readahead { ms: 2 }]);
    }
    calls.push(Play);
    let queue = Queue::default();
    let mut deepest = 0;
    for call in calls {
        queue.push(call, None);
        deepest = deepest.max(queue.len());
    }

    let folded = drain(&queue);

    assert!(deepest <= LANE_CAPACITY, "held {deepest}");
    assert_eq!(folded.iter().rfind(|call| matches!(call, Play | Pause)), Some(&Play));
    assert_eq!(folded.iter().rfind(|call| matches!(call, Mute | Unmute)), Some(&Unmute));
    assert_eq!(folded.iter().rfind(|call| matches!(call, Readahead { .. })), Some(&Readahead { ms: 2 }));
    assert_eq!(folded.iter().rfind(|call| matches!(call, Rate { .. })), Some(&Rate { rate: 2.0 }));
    assert_eq!(folded.iter().filter(|call| matches!(call, Load { .. })).count(), 1);
}
