use super::*;

fn sample(position_ms: u64) -> Sample {
    Sample { position_ms, duration_ms: Some(60_000), is_looping: false, loop_wrapped: false, view_attached: false, frame_rendered: false }
}

fn progress(attempts: u32) -> Progress {
    Progress { has_loaded: true, attempts, is_escalated: false, is_cooldown_elapsed: false, cooldown_cycles: 0 }
}

#[test]
fn inv_session_06_window_delays() {
    let config = WatchdogConfig::default();
    assert_eq!(window_delay_ms(&config, false, false), 12_000);
    assert_eq!(window_delay_ms(&config, true, false), 3_000);
    assert_eq!(window_delay_ms(&config, false, true), 15_000);
    assert_eq!(window_delay_ms(&config, true, true), 15_000);
}

#[test]
fn inv_session_06_clock_advance_needs_more_than_10_ms_or_a_loop_wrap() {
    let config = WatchdogConfig::default();
    assert!(!has_clock_advanced(&config, 1_000, 1_010, false));
    assert!(has_clock_advanced(&config, 1_000, 1_011, false));
    assert!(!has_clock_advanced(&config, 1_000, 900, false));
    assert!(has_clock_advanced(&config, 1_000, 900, true));
}

#[test]
fn inv_session_06_progress_resets_attempts_and_rearms() {
    let verdict = evaluate(&WatchdogConfig::default(), 1_000, &sample(2_000), &progress(3));
    assert_eq!(verdict, Verdict { decision: Decision::ResetAndRearm, attempts: 0 });
}

#[test]
fn inv_session_06_attached_view_requires_a_rendered_frame() {
    let config = WatchdogConfig::default();
    let attached = Sample { view_attached: true, ..sample(2_000) };
    assert_eq!(evaluate(&config, 1_000, &attached, &progress(0)).decision, Decision::Recover);
    let rendered = Sample { frame_rendered: true, ..attached };
    assert_eq!(evaluate(&config, 1_000, &rendered, &progress(0)).decision, Decision::ResetAndRearm);
}

#[test]
fn inv_session_06_near_end_is_healthy_unless_looping() {
    let config = WatchdogConfig::default();
    let near = Sample { duration_ms: Some(10_000), ..sample(9_750) };
    assert_eq!(evaluate(&config, 9_750, &near, &progress(2)).decision, Decision::ResetAndRearm);
    let before = Sample { duration_ms: Some(10_000), ..sample(9_749) };
    assert_eq!(evaluate(&config, 9_749, &before, &progress(2)).decision, Decision::Recover);
    let looping = Sample { is_looping: true, ..near };
    assert_eq!(evaluate(&config, 9_750, &looping, &progress(2)).decision, Decision::Recover);
    let unknown = Sample { duration_ms: None, ..near };
    assert_eq!(evaluate(&config, 9_750, &unknown, &progress(2)).decision, Decision::Recover);
}

#[test]
fn inv_session_06_stall_counts_attempts_and_recovers_while_loaded_else_rearms() {
    let config = WatchdogConfig::default();
    assert_eq!(evaluate(&config, 5, &sample(5), &progress(0)), Verdict { decision: Decision::Recover, attempts: 1 });
    let unloaded = Progress { has_loaded: false, ..progress(2) };
    assert_eq!(evaluate(&config, 5, &sample(5), &unloaded), Verdict { decision: Decision::Rearm, attempts: 3 });
}

#[test]
fn inv_session_06_escalates_once_then_cools_down() {
    let config = WatchdogConfig::default();
    assert_eq!(evaluate(&config, 5, &sample(5), &progress(4)), Verdict { decision: Decision::Escalate, attempts: 0 });
    let used = Progress { is_escalated: true, ..progress(4) };
    assert_eq!(evaluate(&config, 5, &sample(5), &used), Verdict { decision: Decision::EnterCooldown, attempts: 5 });
}

#[test]
fn inv_session_06_terminates_after_two_cooldown_cycles() {
    let config = WatchdogConfig::default();
    let cooled = |cycles| Progress { is_cooldown_elapsed: true, cooldown_cycles: cycles, is_escalated: true, ..progress(5) };
    assert_eq!(evaluate(&config, 5, &sample(5), &cooled(1)), Verdict { decision: Decision::Recover, attempts: 0 });
    assert_eq!(evaluate(&config, 5, &sample(5), &cooled(2)).decision, Decision::Terminate);
}

#[test]
fn inv_session_06_progress_wins_over_an_elapsed_cooldown() {
    let config = WatchdogConfig::default();
    let cooled = Progress { is_cooldown_elapsed: true, cooldown_cycles: 2, ..progress(5) };
    assert_eq!(evaluate(&config, 1_000, &sample(2_000), &cooled).decision, Decision::ResetAndRearm);
}

#[test]
fn inv_session_06_limits_are_configurable() {
    let config = WatchdogConfig { max_attempts: 2, ..WatchdogConfig::default() };
    assert_eq!(evaluate(&config, 5, &sample(5), &progress(1)).decision, Decision::Escalate);
}
