use super::*;
use crate::api::test_support::{eventually, MockFactory};
use crate::api::{MediaError, MediaItem, MediaState, PlayerSlotFactory, TonePixels, TonePreparation};
use crate::ffi::MediaLayer;
use std::sync::Arc;
use super::ingress_harness::*;

fn freed(queue: &Arc<InputQueue>) {
    let garbage = queue.take_garbage();
    let accounts: Vec<(u64, bool)> = garbage.iter().map(queue::Garbage::account).collect();
    drop(garbage);
    queue.freed(&accounts);
}

#[test]
fn inv_ingress_01_updates_of_one_owner_fold_until_a_query_closes_the_snapshot() {
    let queue = InputQueue::new(IngressLimits::default());
    opened(&queue, 1);
    queue.submit(update(1)).unwrap();
    queue.submit(update(1)).unwrap();
    let (_held, _answer) = queue.query(Query::Pool).unwrap();
    queue.submit(update(1)).unwrap();
    queue.submit(update(1)).unwrap();
    queue.submit(Declared::PlaceSeek { owner: 1, fraction: 0.5 }).unwrap();
    queue.submit(update(1)).unwrap();
    queue.submit(update(1)).unwrap();

    assert_eq!(queued(&queue), vec!["open", "update", "query", "update", "seek", "update"]);
}

#[test]
fn inv_ingress_02_events_are_admitted_up_to_the_entry_limit_and_the_rest_is_refused_not_dropped() {
    let queue = InputQueue::new(IngressLimits { entries: 256, owners: 4, bytes: u64::MAX });
    opened(&queue, 1);
    let refused = (0..300).filter(|_| queue.submit(Declared::PlaceSeek { owner: 1, fraction: 0.5 }).is_err()).count();

    assert_eq!(refused, 300 - 255);
    assert_eq!(queued(&queue).iter().filter(|name| **name == "seek").count(), 255);
}

#[test]
fn inv_ingress_03_a_window_is_weighed_by_backing_capacity_including_none_slots_and_empty_pages() {
    let queue = InputQueue::new(IngressLimits { entries: 64, owners: 4, bytes: 64 * 1024 });
    queue.open(1, OwnerKind::Window, Declared::WindowOpen { owner: 1, kind: WindowKind::Video }).unwrap();
    let blanks: Vec<Option<MediaItem>> = vec![None; 100_000];
    let empty_pages: Vec<Vec<MediaItem>> = vec![Vec::new(); 100_000];

    let video = queue.submit(Declared::VideoWindow { owner: 1, active: 0, sources: Some(Arc::new(blanks)) });
    let images = queue.submit(Declared::ImageWindow { owner: 1, active: 0, pages: Some(Arc::new(empty_pages)) });

    assert!(matches!(video, Err(MediaError::Overloaded { .. })), "{video:?}");
    assert!(matches!(images, Err(MediaError::Overloaded { .. })), "{images:?}");
}

#[test]
fn inv_ingress_04_a_failure_detail_and_a_header_heavy_item_are_weighed() {
    let queue = InputQueue::new(IngressLimits { entries: 64, owners: 4, bytes: 4_096 });
    let detail = Declared::Player(PlayerEvent::Failed { slot: 0, generation: 0, status: 1, detail: "x".repeat(10_000) });
    let item = MediaItem { headers: (0..200).map(|index| (format!("h{index}"), "v".repeat(64))).collect(), ..MediaItem::bare_video("https://o.test/a.mp4") };

    assert!(matches!(queue.submit(detail), Err(MediaError::Overloaded { .. })));
    assert!(matches!(queue.open(1, OwnerKind::Place, Declared::PlaceOpen { owner: 1, item }), Err(MediaError::Overloaded { .. })));
    assert_eq!(queue.audit().bytes, 0);
}

#[test]
fn inv_ingress_05_owners_are_bounded() {
    let queue = InputQueue::new(IngressLimits { entries: 64, owners: 2, bytes: u64::MAX });
    opened(&queue, 1);
    opened(&queue, 2);

    let third = queue.open(3, OwnerKind::Place, Declared::PlaceOpen { owner: 3, item: MediaItem::bare_video("https://o.test/c.mp4") });

    assert!(matches!(third, Err(MediaError::Overloaded { .. })));
}

#[test]
fn inv_ingress_06_an_owner_closes_while_the_queue_is_full_and_frees_its_entries_only_after_the_executor_dropped_them() {
    let queue = InputQueue::new(IngressLimits { entries: 4, owners: 4, bytes: u64::MAX });
    opened(&queue, 1);
    for _ in 0..3 {
        queue.submit(Declared::PlaceSeek { owner: 1, fraction: 0.1 }).unwrap();
    }
    assert!(queue.submit(Declared::PlaceSeek { owner: 1, fraction: 0.1 }).is_err());

    queue.release(1);
    queue.release(1);

    let audit = queue.audit();
    assert_eq!((audit.entries, audit.by_owner, audit.folds, audit.garbage), (3, 0, 0, 1));
    assert_eq!(audit.counted, 4, "the dropped command still holds its place until the executor frees it");
    freed(&queue);
    assert_eq!(queue.audit().counted, 3);
    assert!(matches!(queue.submit(update(1)), Err(MediaError::Closed)));
    assert_eq!(queued(&queue), vec!["seek", "seek", "seek", "release"]);
    assert_eq!(queue.audit().counted, 0);
}

#[test]
fn inv_ingress_07_a_cancelled_query_keeps_its_place_and_parameters_until_the_executor_freed_them() {
    let queue = InputQueue::new(IngressLimits { entries: 2, owners: 2, bytes: u64::MAX });
    let (first, _answer) = queue.query(Query::PlayUrl(MediaItem::bare_video("https://o.test/a.mp4"))).unwrap();
    let (second, _other) = queue.query(Query::Pool).unwrap();
    assert!(queue.query(Query::Pool).is_err());

    drop(first);
    assert!(queue.query(Query::Pool).is_err(), "the cancel freed the place before the payload was dropped");
    freed(&queue);
    let taken = queue.next();
    assert!(matches!(taken, queue::Upcoming::Apply(_)));
    drop(taken);
    drop(second);
    freed(&queue);

    assert_eq!(queue.audit(), queue::Audit::default());
}

#[test]
fn inv_ingress_20_repeated_query_and_cancel_cannot_pass_the_limit_while_the_executor_is_held() {
    let queue = InputQueue::new(IngressLimits { entries: 3, owners: 2, bytes: u64::MAX });
    let item = MediaItem::bare_video("https://o.test/a.mp4");

    let admitted = (0..50).filter(|_| queue.query(Query::PlayUrl(item.clone())).map(drop).is_ok()).count();

    assert_eq!(admitted, 3);
    freed(&queue);
    assert_eq!(queue.audit().counted, 0);
}

#[test]
fn inv_ingress_08_repeated_create_update_release_cycles_leave_no_index_behind_with_the_executor_held() {
    let queue = InputQueue::new(IngressLimits { entries: 64, owners: 8, bytes: u64::MAX });
    for round in 0..200u64 {
        let owner = round % 4 + 1;
        opened(&queue, owner);
        queue.submit(update(owner)).unwrap();
        queue.submit(update(owner)).unwrap();
        queue.submit(Declared::PlaceSeek { owner, fraction: 0.5 }).unwrap();
        queue.release(owner);
        assert_eq!(queued(&queue), vec!["seek", "release"]);
        queue.finish_release(owner);
        freed(&queue);
    }

    assert_eq!(queue.audit(), queue::Audit::default());
}

#[test]
fn inv_ingress_09_an_ended_queue_refuses_with_the_reason_and_answers_waiting_work() {
    let queue = InputQueue::new(IngressLimits::default());
    let (_held, answer) = queue.query(Query::Pool).unwrap();

    queue.end(MediaError::Closed);
    let pending = queue.drain();

    assert!(matches!(queue.submit(update(1)), Err(MediaError::Closed)));
    assert!(matches!(queue.query(Query::Pool), Err(MediaError::Closed)));
    assert_eq!(pending.len(), 1);
    drop(answer);
}

#[test]
fn inv_ingress_10_a_held_executor_accepts_declarations_without_running_them_on_the_caller() {
    let dir = tempfile::tempdir().unwrap();
    let factory: Arc<MockFactory> = Arc::default();
    let slots: Arc<dyn PlayerSlotFactory> = factory.clone();
    let pause = Pause::new();
    let layer = MediaLayer::build(config(dir.path(), Some(slots), None), pause.clone()).unwrap();
    let caller = caller();
    let url = "https://o.test/v1.mp4";
    caller.block_on(layer.limits()).unwrap();
    pause.set(true);

    let window = layer.open_video_window().unwrap();
    window.replace(0, vec![Some(MediaItem::bare_video(url))]).unwrap();

    assert!(factory.slot_holding(url).is_none(), "the declaration ran on the caller");
    pause.set(false);
    caller.block_on(eventually(|| factory.slot_holding(url).is_some()));
}

#[test]
fn inv_ingress_30_a_move_costs_the_same_visits_for_any_list_length() {
    let mut visits = Vec::new();
    for length in [20usize, 200, 2000] {
        let queue = InputQueue::new(IngressLimits { entries: 64, owners: 4, bytes: u64::MAX });
        queue.open(1, OwnerKind::Window, Declared::WindowOpen { owner: 1, kind: WindowKind::Video }).unwrap();
        let sources: Vec<Option<MediaItem>> = (0..length).map(|index| Some(MediaItem::bare_video(&format!("https://o.test/{index}.mp4")))).collect();
        queue.submit(Declared::VideoWindow { owner: 1, active: 0, sources: Some(Arc::new(sources)) }).unwrap();
        let before = weight::VISITS.with(|visits| visits.get());

        queue.submit(Declared::VideoWindow { owner: 1, active: 5, sources: None }).unwrap();

        visits.push(weight::VISITS.with(|visits| visits.get()) - before);
    }
    assert_eq!(visits, vec![0, 0, 0]);
}

#[test]
fn inv_ingress_31_closing_with_folded_garbage_writes_off_exactly_to_zero() {
    let queue = InputQueue::new(IngressLimits { entries: 64, owners: 4, bytes: u64::MAX });
    queue.open(1, OwnerKind::Window, Declared::WindowOpen { owner: 1, kind: WindowKind::Video }).unwrap();
    for round in 0..5 {
        let sources: Vec<Option<MediaItem>> = (0..10).map(|index| Some(MediaItem::bare_video(&format!("https://o.test/{round}-{index}.mp4")))).collect();
        queue.submit(Declared::VideoWindow { owner: 1, active: 0, sources: Some(Arc::new(sources)) }).unwrap();
    }
    queue.release(1);
    assert!(queue.audit().garbage > 0);

    let drained = queue.drain();

    assert!(!drained.is_empty());
    assert_eq!(queue.audit(), queue::Audit::default());
    assert!(queue.end_error().is_none(), "the write-off found an accounting mismatch");
}

#[test]
fn inv_ingress_32_a_failed_state_detail_is_weighed() {
    let queue = InputQueue::new(IngressLimits { entries: 64, owners: 4, bytes: 4_096 });
    queue.open(1, OwnerKind::Place, Declared::PlaceOpen { owner: 1, item: MediaItem::bare_video("https://o.test/a.mp4") }).unwrap();

    let verdict = queue.submit(Declared::PlaceState { owner: 1, state: MediaState::Failed { detail: "x".repeat(10_000) } });

    assert!(matches!(verdict, Err(MediaError::Overloaded { .. })));
}

struct DropProbe {
    queue: Arc<InputQueue>,
    unlocked: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl TonePreparation for DropProbe {
    async fn prepare(&self, _source: MediaItem) -> Option<TonePixels> {
        None
    }
}

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.unlocked.store(self.queue.is_unlocked(), std::sync::atomic::Ordering::SeqCst);
    }
}

fn probe(queue: &Arc<InputQueue>) -> (Declared, Arc<std::sync::atomic::AtomicBool>) {
    let unlocked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    (Declared::Preparation(Arc::new(DropProbe { queue: queue.clone(), unlocked: unlocked.clone() })), unlocked)
}

#[test]
fn inv_ingress_33_a_refused_payload_is_dropped_after_the_lock_is_released() {
    let queue = InputQueue::new(IngressLimits { entries: 1, owners: 2, bytes: u64::MAX });
    opened(&queue, 1);
    let (rejected, dropped_unlocked) = probe(&queue);

    let verdict = queue.submit(rejected);

    assert!(matches!(verdict, Err(MediaError::Overloaded { .. })));
    assert!(dropped_unlocked.load(std::sync::atomic::Ordering::SeqCst), "the refused payload was dropped under the lock");
    let tight = InputQueue::new(IngressLimits { entries: 8, owners: 2, bytes: 8 });
    let (by_bytes, bytes_unlocked) = probe(&tight);
    let verdict = tight.submit(by_bytes);
    assert!(matches!(verdict, Err(MediaError::Overloaded { .. })));
    assert!(bytes_unlocked.load(std::sync::atomic::Ordering::SeqCst), "the payload refused by bytes was dropped under the lock");
}
