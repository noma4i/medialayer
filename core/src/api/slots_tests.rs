use super::test_support::{eventually, MockFactory};
use super::*;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Handle;

fn route(url: &str) -> PlaybackRoute {
    PlaybackRoute { url: url.to_string(), headers: Default::default(), generation: Some(0) }
}

#[tokio::test]
async fn commands_reach_the_player_in_order_while_the_caller_never_waits_for_it() {
    let factory: Arc<MockFactory> = Arc::default();
    *factory.load_delay.lock().unwrap() = Duration::from_millis(300);
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));

    let started = Instant::now();
    runner.send(0, SlotCall::Load { key: "a".to_string(), route: route("http://127.0.0.1:1/a") });
    runner.send(0, SlotCall::Play);
    runner.send(0, SlotCall::Mute);
    let waited = started.elapsed();

    assert!(waited < Duration::from_millis(100), "the caller waited {waited:?}");
    eventually(|| factory.slots.lock().unwrap().get(&0).is_some_and(|slot| slot.calls().len() == 3)).await;
    assert_eq!(
        factory.slot(0).calls(),
        vec![SlotCall::Load { key: "a".to_string(), route: route("http://127.0.0.1:1/a") }, SlotCall::Play, SlotCall::Mute]
    );
}

#[tokio::test]
async fn each_slot_gets_one_player_for_its_whole_life_and_a_slow_slot_does_not_hold_another() {
    let factory: Arc<MockFactory> = Arc::default();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));

    for round in 0..50 {
        for slot in 0..3 {
            runner.send(slot, SlotCall::Load { key: format!("k{round}"), route: route("http://127.0.0.1:1/x") });
            runner.send(slot, SlotCall::Unload);
        }
    }

    eventually(|| (0..3).all(|slot| factory.slots.lock().unwrap().get(&slot).is_some_and(|player| player.calls().len() == 100))).await;
    let mut created = factory.created.lock().unwrap().clone();
    created.sort_unstable();
    assert_eq!(created, vec![0, 1, 2]);
}

#[tokio::test]
async fn without_a_factory_commands_are_dropped_and_no_sample_exists() {
    let runner = SlotRunner::new(Handle::current(), None);

    runner.send(0, SlotCall::Play);

    assert_eq!(runner.sample(0).await, None);
}
