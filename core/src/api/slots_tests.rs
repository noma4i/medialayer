use super::lane::LANE_CAPACITY;
use super::test_support::{eventually, MockFactory};
use super::*;
use std::sync::Arc;
use tokio::runtime::Handle;

fn route(url: &str) -> PlaybackRoute {
    PlaybackRoute { url: url.to_string(), headers: Default::default(), generation: Some(0), hls: false, looping: true }
}

#[tokio::test]
async fn inv_api_01_commands_reach_the_player_in_order_while_the_caller_never_waits_for_it() {
    let factory: Arc<MockFactory> = Arc::default();
    factory.gate.close();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));

    runner.send(0, SlotCall::Load { key: "a".to_string(), route: route("http://127.0.0.1:1/a") });
    runner.send(0, SlotCall::Play);
    runner.send(0, SlotCall::Mute);

    eventually(|| factory.gate.entered() == 1).await;
    assert!(factory.slot(0).calls().is_empty());
    factory.gate.open();
    eventually(|| factory.slots.lock().unwrap().get(&0).is_some_and(|slot| slot.calls().len() == 3)).await;
    assert_eq!(
        factory.slot(0).calls(),
        vec![SlotCall::Load { key: "a".to_string(), route: route("http://127.0.0.1:1/a") }, SlotCall::Play, SlotCall::Mute]
    );
}

#[tokio::test]
async fn inv_api_01_a_slow_platform_never_makes_the_queue_grow_and_the_last_source_wins() {
    let factory: Arc<MockFactory> = Arc::default();
    factory.gate.close();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));
    runner.send(0, SlotCall::Load { key: "first".to_string(), route: route("http://127.0.0.1:1/first") });
    eventually(|| factory.gate.entered() == 1).await;

    let mut deepest = 0;
    for index in 0..500 {
        runner.send(0, SlotCall::Load { key: format!("k{index}"), route: route("http://127.0.0.1:1/k") });
        runner.send(0, SlotCall::Preroll);
        runner.send(0, SlotCall::Play);
        runner.send(0, SlotCall::Pause);
        deepest = deepest.max(runner.queued(0));
    }
    runner.send(0, SlotCall::Play);
    deepest = deepest.max(runner.queued(0));
    factory.gate.open();

    eventually(|| factory.slots.lock().unwrap().get(&0).is_some_and(|slot| slot.calls().last() == Some(&SlotCall::Play))).await;
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
    assert!(deepest <= LANE_CAPACITY, "the queue held {deepest} commands");
}

#[tokio::test]
async fn inv_api_01_each_slot_gets_one_player_for_its_whole_life_and_a_slow_slot_does_not_hold_another() {
    let factory: Arc<MockFactory> = Arc::default();
    let runner = SlotRunner::new(Handle::current(), Some(factory.clone()));

    for round in 0..50 {
        for slot in 0..3 {
            runner.send(slot, SlotCall::Load { key: format!("k{round}"), route: route("http://127.0.0.1:1/x") });
            runner.send(slot, SlotCall::Unload);
        }
    }

    eventually(|| (0..3).all(|slot| factory.slots.lock().unwrap().get(&slot).is_some_and(|player| player.calls().last() == Some(&SlotCall::Unload)))).await;
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
    struct Creating {
        entered: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl PlayerSlotFactory for Creating {
        fn create(&self, _slot: u32) -> Arc<dyn PlayerSlot> {
            self.entered.lock().unwrap().take().unwrap().send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            MockFactory::default().create(0)
        }
    }
    for asynchronous in [false, true] {
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let factory = Arc::new(Creating { entered: std::sync::Mutex::new(Some(entered)), release: std::sync::Mutex::new(wait) });
        let runner = Arc::new(SlotRunner::new(Handle::current(), Some(factory)));
        runner.send(0, SlotCall::Play);
        started.await.unwrap();
        let (sampled, answer) = std::sync::mpsc::channel();
        let handle = Handle::current();
        let sampling = std::thread::spawn(move || {
            let sample = if asynchronous { handle.block_on(runner.sample(0)) } else { runner.sample_now(0) };
            sampled.send(sample).unwrap();
        });
        let result = answer.recv_timeout(std::time::Duration::from_secs(1));
        release.send(()).unwrap();
        sampling.join().unwrap();
        assert_eq!(result, Ok(None), "sample waited for create, async={asynchronous}");
    }
}
