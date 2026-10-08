use super::*;

fn stalled() -> Sample {
    Sample { position_ms: 0, duration_ms: Some(60_000), is_looping: false, loop_wrapped: false, view_attached: false, frame_rendered: false }
}

fn moving(position_ms: u64) -> Sample {
    Sample { position_ms, ..stalled() }
}

fn state_with(via_proxy_hls: bool) -> RecoveryState {
    let mut state = RecoveryState::new(WatchdogConfig::default());
    next_action(&mut state, Event::Source { key: "v1".into(), via_proxy_hls }, 0);
    state
}

fn loaded() -> RecoveryState {
    let mut state = state_with(true);
    next_action(&mut state, Event::Loaded, 0);
    state
}

fn delay_of(action: &Action) -> u64 {
    match action {
        Action::Arm { delay_ms } => *delay_ms,
        other => panic!("expected Arm, got {other:?}"),
    }
}

fn play_and_fire(state: &mut RecoveryState, now: &mut u64, sample: Sample) -> Action {
    let delay = delay_of(&next_action(state, Event::Play { position_ms: 0 }, *now));
    *now += delay;
    next_action(state, Event::WatchdogFired(sample), *now)
}

fn run_chain_to_replay(state: &mut RecoveryState, now: u64) {
    assert_eq!(next_action(state, Event::CachePurged, now), Action::RefreshRoute);
    assert_eq!(next_action(state, Event::RouteRefreshed, now), Action::Retry(250));
    let due = Event::RetryDue { resolved_generation: Some(1), current_generation: 1, readiness: Readiness::Ready };
    assert_eq!(next_action(state, due, now), Action::Replay);
}

#[test]
fn watchdog_arms_only_on_play_and_disarms_on_pause() {
    let mut state = loaded();
    assert_eq!(next_action(&mut state, Event::WatchdogFired(stalled()), 12_000), Action::None);
    assert_eq!(next_action(&mut state, Event::Play { position_ms: 0 }, 0), Action::Arm { delay_ms: 12_000 });
    assert_eq!(next_action(&mut state, Event::Pause, 100), Action::Disarm);
    assert_eq!(next_action(&mut state, Event::WatchdogFired(stalled()), 12_000), Action::None);
}

#[test]
fn a_timer_that_fires_before_its_deadline_is_ignored() {
    let mut state = loaded();
    next_action(&mut state, Event::Play { position_ms: 0 }, 1_000);
    assert_eq!(next_action(&mut state, Event::WatchdogFired(stalled()), 12_999), Action::None);
    assert_eq!(next_action(&mut state, Event::WatchdogFired(stalled()), 13_000), Action::Recover);
}

#[test]
fn progress_rearms_with_the_short_window() {
    let mut state = loaded();
    let mut now = 0;
    assert_eq!(play_and_fire(&mut state, &mut now, moving(2_000)), Action::Arm { delay_ms: 3_000 });
    now += 3_000;
    assert_eq!(next_action(&mut state, Event::WatchdogFired(moving(5_000)), now), Action::Arm { delay_ms: 3_000 });
}

#[test]
fn stall_before_load_rearms_instead_of_recovering() {
    let mut state = state_with(true);
    let mut now = 0;
    assert_eq!(play_and_fire(&mut state, &mut now, stalled()), Action::Arm { delay_ms: 12_000 });
}

#[test]
fn five_stalls_escalate_through_purge_refresh_retry_replay() {
    let mut state = loaded();
    let mut now = 0;
    for _ in 0..4 {
        assert_eq!(play_and_fire(&mut state, &mut now, stalled()), Action::Recover);
    }
    assert_eq!(play_and_fire(&mut state, &mut now, stalled()), Action::PurgeCache("v1".into()));
    run_chain_to_replay(&mut state, now);
}

#[test]
fn second_exhaustion_cools_down_and_two_cycles_end_terminal() {
    let mut state = loaded();
    let mut now = 0;
    for _ in 0..4 {
        play_and_fire(&mut state, &mut now, stalled());
    }
    play_and_fire(&mut state, &mut now, stalled());
    run_chain_to_replay(&mut state, now);
    for cycle in 0..2 {
        for _ in 0..4 {
            assert_eq!(play_and_fire(&mut state, &mut now, stalled()), Action::Recover, "cycle {cycle}");
        }
        assert_eq!(play_and_fire(&mut state, &mut now, stalled()), Action::Arm { delay_ms: 15_000 });
        now += 15_000;
        let elapsed = next_action(&mut state, Event::WatchdogFired(stalled()), now);
        if cycle == 0 {
            assert_eq!(elapsed, Action::Recover);
        } else {
            assert_eq!(elapsed, Action::Fail("watchdog terminal".into()));
        }
    }
    assert_eq!(next_action(&mut state, Event::Play { position_ms: 0 }, now), Action::None);
    assert_eq!(next_action(&mut state, Event::WatchdogFired(stalled()), now + 99_999), Action::None);
}

#[test]
fn healthy_progress_resets_the_attempt_budget() {
    let mut state = loaded();
    let mut now = 0;
    for _ in 0..4 {
        play_and_fire(&mut state, &mut now, stalled());
    }
    assert_eq!(play_and_fire(&mut state, &mut now, moving(5_000)), Action::Arm { delay_ms: 3_000 });
    for _ in 0..4 {
        assert_eq!(play_and_fire(&mut state, &mut now, stalled()), Action::Recover);
    }
}

fn drive_to_terminal(state: &mut RecoveryState, now: &mut u64) {
    for _ in 0..5 {
        play_and_fire(state, now, stalled());
    }
    run_chain_to_replay(state, *now);
    for _ in 0..2 {
        for _ in 0..5 {
            play_and_fire(state, now, stalled());
        }
        *now += 15_000;
        next_action(state, Event::WatchdogFired(stalled()), *now);
    }
}

#[test]
fn a_new_source_resets_a_terminal_state() {
    let mut state = loaded();
    let mut now = 0;
    drive_to_terminal(&mut state, &mut now);
    assert_eq!(next_action(&mut state, Event::Play { position_ms: 0 }, now), Action::None);
    next_action(&mut state, Event::Source { key: "v2".into(), via_proxy_hls: true }, 0);
    assert_eq!(next_action(&mut state, Event::Play { position_ms: 0 }, 0), Action::Arm { delay_ms: 12_000 });
}

#[test]
fn startup_http_error_runs_one_recovery_through_the_route_chain() {
    let mut state = state_with(true);
    next_action(&mut state, Event::Play { position_ms: 0 }, 0);
    assert_eq!(next_action(&mut state, Event::HttpError { status: 503 }, 10), Action::RefreshRoute);
    assert_eq!(next_action(&mut state, Event::RouteRefreshed, 20), Action::Retry(250));
    let due = Event::RetryDue { resolved_generation: Some(2), current_generation: 2, readiness: Readiness::Ready };
    assert_eq!(next_action(&mut state, due, 270), Action::Replay);
    assert_eq!(next_action(&mut state, Event::HttpError { status: 503 }, 300), Action::Retry(250));
}

#[test]
fn http_errors_after_load_or_below_400_are_ignored() {
    let mut state = state_with(true);
    next_action(&mut state, Event::Play { position_ms: 0 }, 0);
    assert_eq!(next_action(&mut state, Event::HttpError { status: 304 }, 1), Action::None);
    next_action(&mut state, Event::Loaded, 2);
    assert_eq!(next_action(&mut state, Event::HttpError { status: 500 }, 3), Action::None);
}

#[test]
fn startup_recovery_skips_sources_that_are_not_hls_through_the_proxy() {
    let mut state = state_with(false);
    next_action(&mut state, Event::Play { position_ms: 0 }, 0);
    assert_eq!(next_action(&mut state, Event::HttpError { status: 500 }, 1), Action::Retry(250));
}

#[test]
fn error_retries_follow_the_delay_table_then_fail_with_the_description() {
    let mut state = state_with(false);
    next_action(&mut state, Event::Play { position_ms: 0 }, 0);
    let delays: Vec<Action> = (0..5).map(|_| next_action(&mut state, Event::HttpError { status: 503 }, 1)).collect();
    assert_eq!(delays, [250, 1_000, 3_000, 8_000, 15_000].map(Action::Retry));
    assert_eq!(next_action(&mut state, Event::HttpError { status: 503 }, 2), Action::Fail("HLS load failed: HTTP 503".into()));
}

#[test]
fn failure_without_play_intent_is_not_retried() {
    let mut state = state_with(false);
    assert_eq!(next_action(&mut state, Event::PlaybackFailed { reason: "boom".into() }, 0), Action::Fail("boom".into()));
    next_action(&mut state, Event::Play { position_ms: 0 }, 0);
    next_action(&mut state, Event::Pause, 1);
    assert_eq!(next_action(&mut state, Event::PlaybackFailed { reason: "boom".into() }, 2), Action::Fail("boom".into()));
}

#[test]
fn progress_restarts_the_retry_table() {
    let mut state = loaded();
    next_action(&mut state, Event::Play { position_ms: 0 }, 0);
    next_action(&mut state, Event::PlaybackFailed { reason: "x".into() }, 1);
    assert_eq!(next_action(&mut state, Event::PlaybackFailed { reason: "x".into() }, 2), Action::Retry(1_000));
    let mut now = 0;
    play_and_fire(&mut state, &mut now, moving(5_000));
    assert_eq!(next_action(&mut state, Event::PlaybackFailed { reason: "x".into() }, now), Action::Retry(250));
}

#[test]
fn stale_route_is_refreshed_before_replay_only_when_recovery_is_supported() {
    let stale = Event::RetryDue { resolved_generation: Some(1), current_generation: 2, readiness: Readiness::Ready };
    let mut proxied = loaded();
    assert_eq!(next_action(&mut proxied, stale.clone(), 0), Action::RefreshRoute);
    let mut direct = state_with(false);
    assert_eq!(next_action(&mut direct, stale, 0), Action::Replay);
}

#[test]
fn chain_events_out_of_order_do_nothing() {
    let mut state = loaded();
    assert_eq!(next_action(&mut state, Event::CachePurged, 0), Action::None);
    assert_eq!(next_action(&mut state, Event::RouteRefreshed, 0), Action::None);
}

#[test]
fn foreground_runs_a_pass_and_retries_on_the_six_step_delay_table() {
    assert_eq!(FOREGROUND_STEPS, ["arbitration", "transport", "audio-session", "routes", "ring-projection", "intent"]);
    let mut state = loaded();
    assert_eq!(next_action(&mut state, Event::Foreground, 0), Action::ForegroundPass);
    let mut delays = Vec::new();
    for _ in 0..6 {
        delays.push(next_action(&mut state, Event::ForegroundPassDone { is_ok: false }, 0));
        assert_eq!(next_action(&mut state, Event::ForegroundDue, 0), Action::ForegroundPass);
    }
    assert_eq!(delays, [100, 300, 800, 2_000, 5_000, 10_000].map(Action::ForegroundRetry));
    assert_eq!(next_action(&mut state, Event::ForegroundPassDone { is_ok: false }, 0), Action::None);
    assert_eq!(next_action(&mut state, Event::ForegroundDue, 0), Action::None);
}

#[test]
fn foreground_success_stops_and_background_cancels() {
    let mut state = loaded();
    next_action(&mut state, Event::Foreground, 0);
    assert_eq!(next_action(&mut state, Event::ForegroundPassDone { is_ok: true }, 0), Action::None);
    assert_eq!(next_action(&mut state, Event::ForegroundDue, 0), Action::None);
    next_action(&mut state, Event::Foreground, 0);
    next_action(&mut state, Event::ForegroundPassDone { is_ok: false }, 0);
    assert_eq!(next_action(&mut state, Event::Background, 0), Action::Disarm);
    assert_eq!(next_action(&mut state, Event::ForegroundDue, 0), Action::None);
}

#[test]
fn foreground_resets_watchdog_startup_and_retry_budgets() {
    let mut state = loaded();
    let mut now = 0;
    for _ in 0..4 {
        play_and_fire(&mut state, &mut now, stalled());
    }
    assert_eq!(play_and_fire(&mut state, &mut now, stalled()), Action::PurgeCache("v1".into()));
    next_action(&mut state, Event::Foreground, now);
    for _ in 0..4 {
        assert_eq!(play_and_fire(&mut state, &mut now, stalled()), Action::Recover);
    }
    assert_eq!(play_and_fire(&mut state, &mut now, stalled()), Action::PurgeCache("v1".into()));
}
