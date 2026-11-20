use super::*;
use crate::seeded::Dice;

fn start() -> OutgoingState {
    outgoing_declare("k".into(), OutgoingKind::Image, true, None)
}
fn up(state: &OutgoingState, upload: OutgoingUpload) -> OutgoingState {
    outgoing_next(Some(state.clone()), OutgoingEvent::Upload { upload })
}
fn rem(state: &OutgoingState, url: &str) -> OutgoingState {
    outgoing_next(Some(state.clone()), OutgoingEvent::Remote { remote: Some(url.into()) })
}
fn res(state: &OutgoingState, result: RemoteResult) -> OutgoingState {
    outgoing_next(Some(state.clone()), OutgoingEvent::RemoteResult { result })
}
fn sending(progress: f64) -> OutgoingPhase {
    OutgoingPhase::Sending { progress }
}
fn fetching(progress: f64) -> OutgoingPhase {
    OutgoingPhase::FetchingRemote { progress }
}
fn failed_remote() -> RemoteResult {
    RemoteResult::Failed { status: Some(404), detail: "gone".into() }
}

#[test]
fn inv_outgoing_01_the_happy_path_walks_the_table() {
    let mut state = start();
    assert_eq!(state.phase, sending(0.0));
    state = up(&state, OutgoingUpload::Sending { progress: 0.4 });
    assert_eq!(state.phase, sending(0.4));
    state = up(&state, OutgoingUpload::Sent);
    assert_eq!(state.phase, OutgoingPhase::AwaitingRemote);
    state = rem(&state, "https://a/1");
    assert_eq!(state.phase, fetching(0.0));
    state = res(&state, RemoteResult::Fetching { progress: 0.5 });
    assert_eq!(state.phase, fetching(0.5));
    state = res(&state, RemoteResult::Complete);
    assert_eq!(state.phase, OutgoingPhase::Ready);
}

#[test]
fn inv_outgoing_02_upload_progress_is_monotonic_and_duplicates_fold() {
    let mut state = up(&start(), OutgoingUpload::Sending { progress: 0.6 });
    state = up(&state, OutgoingUpload::Sending { progress: 0.3 });
    assert_eq!(state.phase, sending(0.6));
    state = up(&state, OutgoingUpload::Sending { progress: 0.6 });
    assert_eq!(state.phase, sending(0.6));
    state = up(&state, OutgoingUpload::Sent);
    state = up(&state, OutgoingUpload::Sending { progress: 0.9 });
    assert_eq!(state.phase, OutgoingPhase::AwaitingRemote);
}

#[test]
fn inv_outgoing_03_a_remote_url_implies_sent_when_the_upload_entry_disappears() {
    let state = rem(&up(&start(), OutgoingUpload::Sending { progress: 0.7 }), "https://a/1");
    assert_eq!(state.inputs.upload, OutgoingUpload::Sent);
    assert_eq!(state.phase, fetching(0.0));
    let failed = up(&start(), OutgoingUpload::Failed { reason: "net".into() });
    let still = rem(&failed, "https://a/1");
    assert_eq!(still.inputs.upload, OutgoingUpload::Failed { reason: "net".into() });
}

#[test]
fn inv_outgoing_04_a_remote_failure_is_final_until_retry() {
    let mut state = rem(&start(), "https://a/1");
    state = res(&state, failed_remote());
    let expected = OutgoingPhase::Failed { reason: OutgoingFailure::Remote { status: Some(404), detail: "gone".into() } };
    assert_eq!(state.phase, expected);
    state = res(&state, RemoteResult::Fetching { progress: 0.5 });
    state = res(&state, RemoteResult::Complete);
    assert_eq!(state.phase, expected);
    state = outgoing_next(Some(state), OutgoingEvent::Retry);
    assert_eq!(state.phase, fetching(0.0));
    state = res(&state, RemoteResult::Complete);
    assert_eq!(state.phase, OutgoingPhase::Ready);
}

#[test]
fn inv_outgoing_05_retry_works_only_from_a_remote_failure() {
    let state = rem(&start(), "https://a/1");
    assert_eq!(outgoing_next(Some(state.clone()), OutgoingEvent::Retry).phase, state.phase);
    let failed = up(&start(), OutgoingUpload::Failed { reason: "net".into() });
    assert_eq!(outgoing_next(Some(failed.clone()), OutgoingEvent::Retry).phase, failed.phase);
    let ready = res(&state, RemoteResult::Complete);
    assert_eq!(outgoing_next(Some(ready), OutgoingEvent::Retry).phase, OutgoingPhase::Ready);
}

#[test]
fn inv_outgoing_06_upload_failure_shows_failed_unless_ready() {
    let failed = up(&start(), OutgoingUpload::Failed { reason: "net".into() });
    assert_eq!(failed.phase, OutgoingPhase::Failed { reason: OutgoingFailure::Upload { reason: "net".into() } });
    let resumed = up(&failed, OutgoingUpload::Sending { progress: 0.1 });
    assert_eq!(resumed.phase, sending(0.1));
    let ready = res(&rem(&start(), "https://a/1"), RemoteResult::Complete);
    assert_eq!(up(&ready, OutgoingUpload::Failed { reason: "late".into() }).phase, OutgoingPhase::Ready);
}

#[test]
fn inv_outgoing_07_ready_is_final_for_the_same_remote_and_a_new_remote_restarts() {
    let ready = res(&rem(&start(), "https://a/1"), RemoteResult::Complete);
    assert_eq!(rem(&ready, "https://a/1").phase, OutgoingPhase::Ready);
    assert_eq!(res(&ready, RemoteResult::NotReady).phase, OutgoingPhase::Ready);
    assert_eq!(res(&ready, failed_remote()).phase, OutgoingPhase::Ready);
    assert_eq!(outgoing_next(Some(ready.clone()), OutgoingEvent::Remote { remote: None }).phase, OutgoingPhase::Ready);
    assert_eq!(rem(&ready, "https://a/2").phase, fetching(0.0));
}

#[test]
fn inv_outgoing_08_the_view_follows_the_phase() {
    let mut state = start();
    let view = outgoing_view(state.clone());
    assert!(view.shows_local && view.shows_loader && !view.can_open && view.reask_after_ms.is_none());
    state = rem(&state, "https://a/1");
    state = res(&state, RemoteResult::NotReady);
    assert_eq!(outgoing_view(state.clone()).reask_after_ms, Some(5000));
    state = res(&state, RemoteResult::Fetching { progress: 0.2 });
    assert_eq!(outgoing_view(state.clone()).reask_after_ms, None);
    state = res(&state, failed_remote());
    let view = outgoing_view(state.clone());
    assert!(view.shows_local && !view.shows_loader && !view.can_open);
    state = outgoing_next(Some(state), OutgoingEvent::Retry);
    state = res(&state, RemoteResult::Complete);
    let view = outgoing_view(state);
    assert!(!view.shows_local && !view.shows_loader && view.can_open);
}

#[test]
fn inv_outgoing_09_no_loader_without_a_local() {
    let state = outgoing_declare("k".into(), OutgoingKind::Image, false, Some("LEHV6n".into()));
    let view = outgoing_view(state);
    assert!(view.shows_local && !view.shows_loader);
}

#[test]
fn inv_outgoing_10_gc_rule() {
    let day = 24 * 60 * 60 * 1000u64;
    assert!(outgoing_stale(0, 0, 6 * day, false));
    assert!(!outgoing_stale(0, 6 * day, 6 * day + 1000, false));
    assert!(outgoing_stale(0, 7 * day, 7 * day + day / 2, false));
    assert!(outgoing_stale(0, 7 * day, 7 * day + 1, false));
    assert!(outgoing_stale(0, 2 * day, 2 * day + day + 1, false));
    assert!(!outgoing_stale(0, 2 * day, 3 * day, false));
    assert!(!outgoing_stale(0, 0, 100 * day, true));
    assert!(!outgoing_stale(5, 5, 1, false));
    assert_eq!(outgoing_fetch_slots(), 2);
}

#[test]
fn inv_outgoing_19_a_late_local_is_taken() {
    let mut state = outgoing_declare("k".into(), OutgoingKind::Image, false, Some("LEHV6n".into()));
    assert!(!outgoing_view(state.clone()).shows_loader);
    state = outgoing_next(Some(state), OutgoingEvent::Local { has_local: true });
    assert!(state.inputs.has_local);
    assert!(outgoing_view(state.clone()).shows_loader);
    state = outgoing_next(Some(state), OutgoingEvent::Local { has_local: false });
    assert!(state.inputs.has_local, "a local is never taken back");
    state = outgoing_next(Some(state), OutgoingEvent::Remote { remote: Some("https://a/0".into()) });
    state = outgoing_next(Some(state), OutgoingEvent::RemoteResult { result: RemoteResult::Complete });
    assert!(state.inputs.has_local && !outgoing_view(state).shows_loader);
}

fn random_event(dice: &mut Dice) -> OutgoingEvent {
    let progress = dice.below(101) as f64 / 100.0;
    let url = format!("https://a/{}", dice.below(3));
    match dice.below(11) {
        0 | 1 => OutgoingEvent::Upload { upload: OutgoingUpload::Sending { progress } },
        2 => OutgoingEvent::Upload { upload: OutgoingUpload::Sent },
        3 => OutgoingEvent::Upload { upload: OutgoingUpload::Failed { reason: "e".into() } },
        4 => OutgoingEvent::Remote { remote: Some(url) },
        5 => OutgoingEvent::Remote { remote: None },
        6 => OutgoingEvent::RemoteResult { result: RemoteResult::NotReady },
        7 => OutgoingEvent::RemoteResult { result: RemoteResult::Fetching { progress } },
        8 => OutgoingEvent::RemoteResult {
            result: if dice.percent(50) { RemoteResult::Complete } else { failed_remote() },
        },
        9 => OutgoingEvent::Local { has_local: dice.percent(50) },
        _ => OutgoingEvent::Retry,
    }
}

#[test]
fn inv_outgoing_11_random_sequences_keep_the_rules() {
    let mut dice = Dice::new("inv_outgoing_11_random_sequences_keep_the_rules");
    for _ in 0..2000 {
        let mut state = start();
        for _ in 0..40 {
            let event = random_event(&mut dice);
            let before = state.clone();
            state = outgoing_next(Some(before.clone()), event.clone());
            let view = outgoing_view(state.clone());
            let same_remote = before.inputs.remote == state.inputs.remote;

            if before.phase == OutgoingPhase::Ready && same_remote {
                assert_eq!(state.phase, OutgoingPhase::Ready, "ready went back on {event:?}");
            }
            if let (OutgoingPhase::Sending { progress: a }, OutgoingPhase::Sending { progress: b }) = (&before.phase, &state.phase) {
                assert!(b >= a, "upload progress went back on {event:?}");
            }
            if let (OutgoingPhase::FetchingRemote { progress: a }, OutgoingPhase::FetchingRemote { progress: b }) = (&before.phase, &state.phase) {
                if same_remote && event != OutgoingEvent::Retry {
                    assert!(b >= a, "fetch progress went back on {event:?}");
                }
            }
            if before.inputs.remote.is_some() {
                assert!(state.inputs.remote.is_some(), "a known remote was taken back");
            }
            assert_eq!(view.shows_local, state.phase != OutgoingPhase::Ready);
            assert_eq!(view.can_open, state.phase == OutgoingPhase::Ready);
            if view.reask_after_ms.is_some() {
                assert_eq!(state.inputs.remote_result, Some(RemoteResult::NotReady));
            }
            if before.inputs.has_local {
                assert!(state.inputs.has_local, "a local was taken back on {event:?}");
            }
            if view.shows_loader {
                assert!(state.inputs.has_local);
            }
        }
    }
}
