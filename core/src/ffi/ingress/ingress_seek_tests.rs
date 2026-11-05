use super::*;
use crate::api::test_support::{eventually, MockFactory};
use crate::api::{MediaError, MediaItem, PlayerSlotFactory, SlotCall};
use crate::ffi::MediaLayer;
use std::sync::Arc;
use super::ingress_harness::*;

fn key_seek(key: &str, fraction: f64) -> Declared {
    Declared::KeySeek { key: key.to_string(), fraction }
}

fn pending_fractions(queue: &Arc<InputQueue>) -> Vec<(String, f64)> {
    let mut seen = Vec::new();
    while let queue::Upcoming::Apply(popped) = queue.next() {
        if let Declared::KeySeek { key, fraction } = popped.command {
            seen.push((key, fraction));
        }
    }
    seen
}

#[test]
fn inv_ingress_37_key_seeks_of_one_key_fold_into_the_last_fraction_and_other_keys_stay_apart() {
    let queue = InputQueue::new(IngressLimits::default());
    queue.submit(key_seek("a", 0.1)).unwrap();
    queue.submit(key_seek("b", 0.9)).unwrap();
    queue.submit(key_seek("a", 0.2)).unwrap();
    queue.submit(key_seek("a", 0.3)).unwrap();

    assert_eq!(pending_fractions(&queue), vec![("a".to_string(), 0.3), ("b".to_string(), 0.9)]);
}

fn pending_muted_all(queue: &Arc<InputQueue>) -> Vec<bool> {
    let mut seen = Vec::new();
    while let queue::Upcoming::Apply(popped) = queue.next() {
        if let Declared::MutedAll(muted) = popped.command {
            seen.push(muted);
        }
    }
    seen
}

#[test]
fn inv_ingress_42_mute_all_calls_fold_into_the_last_value_until_they_are_drained() {
    let queue = InputQueue::new(IngressLimits::default());
    queue.submit(Declared::MutedAll(true)).unwrap();
    queue.submit(Declared::MutedAll(false)).unwrap();

    assert_eq!(pending_muted_all(&queue), vec![false]);
}

#[test]
fn inv_ingress_38_a_key_seek_after_a_barrier_never_folds_into_one_before_it_and_keeps_its_order() {
    let queue = InputQueue::new(IngressLimits::default());
    opened(&queue, 1);
    queue.submit(key_seek("a", 0.1)).unwrap();
    queue.submit(update(1)).unwrap();
    let (_held, _answer) = queue.query(Query::Pool).unwrap();
    queue.submit(key_seek("a", 0.2)).unwrap();
    queue.submit(key_seek("a", 0.3)).unwrap();

    assert_eq!(queued(&queue), vec!["open", "key-seek", "update", "query", "key-seek"]);
}

#[test]
fn inv_ingress_39_a_key_seek_is_weighed_and_refused_as_overloaded_at_the_limits() {
    let heavy = InputQueue::new(IngressLimits { entries: 64, owners: 4, bytes: 4_096 });
    let full = InputQueue::new(IngressLimits { entries: 1, owners: 4, bytes: u64::MAX });
    full.submit(key_seek("a", 0.1)).unwrap();

    let long = heavy.submit(key_seek(&"k".repeat(10_000), 0.5));
    let beyond = full.submit(key_seek("b", 0.1));
    let folded = full.submit(key_seek("a", 0.2));

    assert!(matches!(long, Err(MediaError::Overloaded { .. })), "{long:?}");
    assert!(matches!(beyond, Err(MediaError::Overloaded { .. })), "{beyond:?}");
    assert!(folded.is_ok());
    assert_eq!(heavy.audit().bytes, 0);
}

#[test]
fn inv_ingress_40_a_key_seek_of_an_unloaded_key_applies_when_the_key_loads() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/v1.mp4";

    let creating = factory.create_gate.hold();

    layer.seek(url.to_string(), 0.5).unwrap();
    let ring = layer.open_video_window().unwrap();
    ring.replace(0, vec![Some(MediaItem::bare_video(url))]).unwrap();
    let place = layer.open_place(MediaItem::bare_video(url)).unwrap();
    place.update(PlaceInput { playing: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.create_gate.entered() == 1 && layer.queue().audit().counted == 0));
    drop(creating);
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let (index, slot) = factory.slot_holding(url).unwrap();
    layer.player_events().ready(index, slot.generation());

    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Seek { position_ms: 30_000 })));
}

#[test]
fn inv_ingress_41_a_closed_core_refuses_the_key_seek_to_the_caller() {
    let dir = tempfile::tempdir().unwrap();
    let layer = MediaLayer::build(config(dir.path(), None, None), Pause::new()).unwrap();
    let caller = caller();
    caller.block_on(layer.await_closed());

    let refused = layer.seek("https://o.test/v1.mp4".to_string(), 0.5);

    assert!(matches!(refused, Err(MediaError::Closed)), "{refused:?}");
}

#[test]
fn inv_ingress_44_a_note_that_ended_and_was_rewound_by_its_closing_place_returns_at_the_start_when_mounted_again() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/note.m4a";
    let note = MediaItem { looping: false, ..MediaItem::bare(url, crate::api::Kind::Audio) };

    let place = layer.open_place(note.clone()).unwrap();
    place.update(PlaceInput { playing: true, visible: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let (index, slot) = factory.slot_holding(url).unwrap();
    {
        let mut sample = slot.sample.lock().unwrap();
        sample.position_ms = 60_000;
        sample.duration_ms = Some(60_000);
    }
    layer.player_events().ready(index, slot.generation());
    layer.player_events().ended(index, slot.generation());
    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Seek { position_ms: 0 })));

    place.seek(0.0).unwrap();
    place.release();
    caller.block_on(eventually(|| factory.slot_holding(url).is_none()));
    let again = layer.open_place(note).unwrap();
    again.update(PlaceInput { playing: true, visible: true, ..PlaceInput::default() }).unwrap();

    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let (_, slot) = factory.slot_holding(url).unwrap();
    caller.block_on(eventually(|| layer.queue().audit().counted == 0));
    let calls = slot.calls();
    let load = calls.iter().rposition(|call| matches!(call, SlotCall::Load { .. })).unwrap();
    assert!(!calls[load..].iter().any(|call| matches!(call, SlotCall::Seek { position_ms } if *position_ms > 0)), "{calls:?}");
}

#[test]
fn inv_ingress_45_a_seek_submitted_just_before_the_release_of_its_place_is_kept_for_the_next_place_of_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), Pause::new()).unwrap();
    let caller = caller();
    let url = "https://o.test/note.m4a";
    let note = MediaItem { looping: false, ..MediaItem::bare(url, crate::api::Kind::Audio) };
    let place = layer.open_place(note.clone()).unwrap();
    place.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let (index, slot) = factory.slot_holding(url).unwrap();
    slot.sample.lock().unwrap().duration_ms = Some(60_000);
    layer.player_events().ready(index, slot.generation());

    place.seek(0.5).unwrap();
    place.release();
    caller.block_on(eventually(|| factory.slot_holding(url).is_none()));
    let again = layer.open_place(note).unwrap();
    again.update(PlaceInput { visible: true, ..PlaceInput::default() }).unwrap();

    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
    let (_, slot) = factory.slot_holding(url).unwrap();
    caller.block_on(eventually(|| slot.calls().contains(&SlotCall::Seek { position_ms: 30_000 })));
}
