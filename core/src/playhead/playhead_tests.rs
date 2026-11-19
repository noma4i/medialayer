use super::*;

const DURATION: u64 = 10_000;

fn report(position_ms: u64, at_ms: u64, rate: f64, playing: bool, jump: bool) -> PlayheadReport {
    PlayheadReport { position_ms, duration_ms: DURATION, at_ms, rate, playing, jump, settling: false }
}

fn playing_from(position_ms: u64, at_ms: u64, rate: f64) -> PlayheadAnchor {
    playhead_next(None, report(position_ms, at_ms, rate, true, false))
}

fn near(left: f64, right: f64) {
    assert!((left - right).abs() < 1e-9, "{left} != {right}");
}

#[test]
fn inv_playhead_01_a_playing_report_anchors_a_position_that_advances_by_the_player_rate() {
    near(playhead_at(playing_from(2000, 1000, 1.0), 1500), 0.25);
    near(playhead_at(playing_from(2000, 1000, 2.0), 1500), 0.3);
}

#[test]
fn inv_playhead_02_a_report_behind_the_shown_position_by_up_to_a_second_does_not_step_back() {
    let shown = playing_from(2000, 1000, 1.0);

    let next = playhead_next(Some(shown), report(1910, 1250, 1.0, true, false));

    near(playhead_at(next, 1250), 0.225);
    near(playhead_at(next, 1750), 0.275);
}

#[test]
fn inv_playhead_03_a_report_far_behind_the_shown_position_snaps_to_the_player() {
    let shown = playing_from(2000, 1000, 1.0);

    let next = playhead_next(Some(shown), report(250, 1250, 1.0, true, false));

    near(playhead_at(next, 1250), 0.025);
}

#[test]
fn inv_playhead_04_a_report_ahead_of_the_shown_position_moves_it_forward() {
    let shown = playing_from(2000, 1000, 1.0);

    let next = playhead_next(Some(shown), report(2500, 1250, 1.0, true, false));

    near(playhead_at(next, 1250), 0.25);
}

#[test]
fn inv_playhead_05_a_seek_takes_the_reported_position_even_when_it_is_behind() {
    let shown = playing_from(2000, 1000, 1.0);

    let next = playhead_next(Some(shown), report(500, 1250, 1.0, true, true));

    near(playhead_at(next, 1250), 0.05);
}

#[test]
fn inv_playhead_06_a_pause_freezes_the_position_already_shown() {
    let shown = playing_from(2000, 1000, 1.0);

    let next = playhead_next(Some(shown), report(2100, 1250, 1.0, false, false));

    near(playhead_at(next, 1250), 0.225);
    near(playhead_at(next, 5000), 0.225);
}

#[test]
fn inv_playhead_07_a_rate_change_continues_from_the_shown_position_at_the_new_rate() {
    let shown = playing_from(2000, 1000, 1.0);

    let next = playhead_next(Some(shown), report(2250, 1250, 2.0, true, false));

    near(playhead_at(next, 1250), 0.225);
    near(playhead_at(next, 1750), 0.325);
}

#[test]
fn inv_playhead_08_the_position_stops_at_the_end() {
    near(playhead_at(playing_from(9000, 1000, 1.0), 9000), 1.0);
}

#[test]
fn inv_playhead_09_a_report_while_a_seek_is_in_flight_keeps_the_position_already_shown() {
    let shown = playhead_next(Some(playing_from(2000, 1000, 1.0)), report(8000, 1100, 1.0, true, true));
    let stale = PlayheadReport { settling: true, ..report(2200, 1300, 1.0, true, false) };

    let next = playhead_next(Some(shown), stale);

    near(playhead_at(next, 1300), 0.82);
}

#[test]
fn inv_playhead_10_a_report_with_an_unknown_duration_freezes_the_shown_position_instead_of_resetting_it() {
    let shown = playing_from(2000, 1000, 1.0);
    let unknown = PlayheadReport { duration_ms: 0, position_ms: 0, ..report(0, 1250, 1.0, false, false) };

    let next = playhead_next(Some(shown), unknown);

    near(playhead_at(next, 1250), 0.225);
    near(playhead_at(next, 5000), 0.225);
}

#[test]
fn inv_playhead_11_a_resume_after_a_long_pause_continues_from_the_frozen_position() {
    let shown = playing_from(2000, 1000, 1.0);
    let paused = playhead_next(Some(shown), report(2250, 1250, 1.0, false, false));

    let resumed = playhead_next(Some(paused), report(2250, 6000, 1.0, true, false));

    near(playhead_at(paused, 6000), 0.225);
    near(playhead_at(resumed, 6500), 0.275);
}

#[test]
fn inv_playhead_15_take_ticks_every_100_ms_never_move_the_ring_back_and_reach_one_at_the_limit() {
    let limit = 3000u64;
    let take = |start: u64, tick_ms: u64, late: u64| PlayheadReport {
        position_ms: tick_ms.saturating_sub(late),
        duration_ms: limit,
        at_ms: start + tick_ms,
        rate: 1.0,
        playing: true,
        jump: tick_ms == 0,
        settling: false,
    };
    let mut shown = None;
    let mut previous = 0.0;
    let mut samples = 0;
    for tick in (0..=limit).step_by(100) {
        let late = if tick % 300 == 100 { 30 } else if tick % 200 == 0 { 0 } else { 15 };
        let next = playhead_next(shown, take(5000, tick, late));
        for now in (5000 + tick..5000 + tick + 100).step_by(16) {
            let value = playhead_at(next, now);
            assert!(value >= previous - 1e-12, "ring moved back at {now}: {value} < {previous}");
            previous = value;
            samples += 1;
        }
        shown = Some(next);
    }
    assert!(samples > 150);
    near(playhead_at(shown.unwrap(), 5000 + limit), 1.0);

    let again = playhead_next(shown, take(20_000, 0, 0));
    near(playhead_at(again, 20_000), 0.0);
}
