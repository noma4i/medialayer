use super::lane::LANE_CAPACITY;
use super::test_support::{eventually, MockFactory, MockSlot};
use super::*;
use std::sync::Arc;
use tokio::runtime::Handle;

fn route(url: &str) -> PlaybackRoute {
    PlaybackRoute { url: url.to_string(), headers: Default::default(), generation: Some(0), hls: false, looping: true }
}

fn load(key: &str, generation: u64) -> SlotCall {
    SlotCall::Load { key: key.to_string(), route: route(&format!("https://{key}")), generation }
}

fn applied(factory: &MockFactory, slot: u32) -> Vec<SlotCall> {
    factory.slots.lock().unwrap().get(&slot).map(|player| player.calls()).unwrap_or_default()
}

#[tokio::test]
async fn inv_api_42_a_command_is_done_only_after_the_native_side_applied_it() {
    let factory: Arc<MockFactory> = Arc::default();
    let held = factory.gate.hold();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));
    for call in [load("a", 1), SlotCall::Play, SlotCall::Mute] {
        runner.send(0, call);
    }
    eventually(|| factory.gate.entered() == 1).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }

    assert!(!runner.is_idle(), "the runner is idle while the first command is not applied");
    assert_eq!(factory.slot(0).applies_started(), 1, "the next command started before the first one was applied");
    drop(held);
    eventually(|| runner.is_idle()).await;
    assert_eq!(factory.slot(0).calls(), vec![load("a", 1), SlotCall::Play, SlotCall::Mute]);
    assert_eq!(factory.slot(0).peak_applies(), 1);
}

#[tokio::test]
async fn inv_api_01_new_load_during_creation_replaces_unstarted_commands() {
    let factory: Arc<MockFactory> = Arc::default();
    let held = factory.create_gate.hold();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));
    runner.send(0, load("old", 1));
    runner.send(0, SlotCall::Play);
    eventually(|| factory.create_gate.entered() == 1).await;
    runner.send(0, load("new", 2));
    runner.send(0, SlotCall::Mute);
    drop(held);
    eventually(|| runner.is_idle()).await;
    assert_eq!(factory.slot(0).calls(), vec![load("new", 2), SlotCall::Mute]);
}

#[tokio::test]
async fn inv_api_01_commands_reach_the_player_in_order_while_the_caller_never_waits_for_it() {
    let factory: Arc<MockFactory> = Arc::default();
    let held = factory.gate.hold();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));

    runner.send(0, load("a", 1));
    runner.send(0, SlotCall::Play);
    runner.send(0, SlotCall::Mute);

    eventually(|| factory.gate.entered() == 1).await;
    assert!(factory.slot(0).calls().is_empty());
    drop(held);
    eventually(|| applied(&factory, 0).len() == 3).await;
    assert_eq!(factory.slot(0).calls(), vec![load("a", 1), SlotCall::Play, SlotCall::Mute]);
}

#[tokio::test]
async fn inv_api_42_a_burst_while_the_native_side_is_held_keeps_the_queue_bounded_and_one_command_in_flight() {
    let factory: Arc<MockFactory> = Arc::default();
    let held = factory.gate.hold();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));
    runner.send(0, load("first", 1));
    runner.send(0, SlotCall::Play);
    eventually(|| factory.gate.entered() == 1).await;

    let mut deepest = 0;
    for index in 0..500u64 {
        runner.send(0, load(&format!("k{index}"), 2 + index));
        runner.send(0, SlotCall::Preroll);
        runner.send(0, SlotCall::Play);
        runner.send(0, SlotCall::Pause);
        deepest = deepest.max(runner.queued(0));
    }
    runner.send(0, SlotCall::Play);
    deepest = deepest.max(runner.queued(0));
    assert_eq!(factory.slot(0).applies_started(), 1, "a command started while the first one was held");
    drop(held);

    eventually(|| applied(&factory, 0).last() == Some(&SlotCall::Play)).await;
    let loaded: Vec<String> = factory
        .slot(0)
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            SlotCall::Load { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    assert_eq!(loaded, vec!["first".to_string(), "k499".to_string()]);
    assert!(matches!(&factory.slot(0).calls()[1], SlotCall::Load { key, generation, .. } if key == "k499" && *generation == 501));
    assert!(deepest <= LANE_CAPACITY, "the queue held {deepest} commands");
    assert_eq!(factory.slot(0).peak_applies(), 1);
}

#[tokio::test]
async fn inv_api_42_closing_the_runner_cancels_the_held_apply_and_runs_nothing_after_it() {
    let factory: Arc<MockFactory> = Arc::default();
    let held = factory.gate.hold();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));
    runner.send(0, load("a", 1));
    runner.send(0, SlotCall::Play);
    eventually(|| factory.gate.entered() == 1).await;
    let slot: Arc<MockSlot> = factory.slot(0);

    let mut task = runner.observe_task(0).expect("slot exists");
    drop(runner);
    let _ = task.join().await;
    assert_eq!(slot.cancelled_applies(), 1);
    drop(held);
    assert_eq!(Arc::strong_count(&slot), 2);
    assert!(slot.calls().is_empty(), "a command ran after the close: {:?}", slot.calls());
    assert_eq!(slot.applies_started(), 1);
}

#[tokio::test]
async fn inv_api_42_a_held_seek_blocks_a_play_but_gives_way_to_a_new_source() {
    let factory: Arc<MockFactory> = Arc::default();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));
    runner.send(0, load("a", 1));
    eventually(|| runner.is_idle()).await;
    let slot = factory.slot(0);

    let held = factory.seek_gate.hold();
    runner.send(0, SlotCall::Seek { position_ms: 0 });
    eventually(|| factory.seek_gate.entered() == 1).await;
    runner.send(0, SlotCall::Play);
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(slot.calls(), vec![load("a", 1)], "the play passed the held seek");
    assert_eq!(slot.cancelled_applies(), 0, "a play cancelled the seek");
    drop(held);
    eventually(|| runner.is_idle()).await;
    assert_eq!(slot.calls(), vec![load("a", 1), SlotCall::Seek { position_ms: 0 }, SlotCall::Play]);

    let held = factory.seek_gate.hold();
    runner.send(0, SlotCall::Seek { position_ms: 5 });
    eventually(|| factory.seek_gate.entered() == 2).await;
    runner.send(0, SlotCall::Play);
    runner.send(0, SlotCall::Unload);
    eventually(|| slot.calls().last() == Some(&SlotCall::Unload)).await;
    assert_eq!(slot.cancelled_applies(), 1, "the held seek of the old source was not dropped");
    assert_eq!(slot.calls().iter().filter(|call| matches!(call, SlotCall::Seek { position_ms: 5 })).count(), 0);
    drop(held);
}

#[tokio::test]
async fn inv_api_01_each_slot_gets_one_player_for_its_whole_life_and_a_slow_slot_does_not_hold_another() {
    let factory: Arc<MockFactory> = Arc::default();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));

    for slot in 0..3 {
        runner.send(slot, SlotCall::Mute);
    }
    for round in 0..50u64 {
        for slot in 0..3 {
            runner.send(slot, load(&format!("k{round}"), round));
            runner.send(slot, SlotCall::Unload);
        }
    }

    eventually(|| (0..3).all(|slot| applied(&factory, slot).last() == Some(&SlotCall::Unload))).await;
    let mut created = factory.created.lock().unwrap().clone();
    created.sort_unstable();
    assert_eq!(created, vec![0, 1, 2]);
}

#[tokio::test]
async fn inv_api_01_without_a_factory_commands_are_dropped_and_no_sample_exists() {
    let runner = SlotRunner::new(Handle::current(), None);

    runner.send(0, SlotCall::Play);

    assert_eq!(runner.sample(0).await, None);
}

#[tokio::test]
async fn inv_api_33_sample_does_not_wait_for_create() {
    let factory: Arc<MockFactory> = Arc::default();
    let held = factory.create_gate.hold();
    let runner = Arc::new(SlotRunner::new(Handle::current(), Some(factory.clone())));
    runner.send(0, SlotCall::Play);
    eventually(|| factory.create_gate.entered() == 1).await;

    let (sampled, answer) = std::sync::mpsc::channel();
    let instant = runner.clone();
    let sampling = std::thread::spawn(move || sampled.send(instant.sample_now(0)).unwrap());
    let result = answer.recv_timeout(std::time::Duration::from_secs(1));
    sampling.join().unwrap();
    let deferred = tokio::time::timeout(std::time::Duration::from_secs(1), runner.sample(0)).await;

    drop(held);
    assert_eq!(result, Ok(None), "sample_now waited for create");
    assert_eq!(deferred, Ok(None), "sample waited for create");
}
