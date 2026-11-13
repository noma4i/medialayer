use super::*;

const DURATION: u64 = 10_000;

fn report(position_ms: u64, at_ms: u64, rate: f64, playing: bool, jump: bool) -> PlayheadReport {
    PlayheadReport { position_ms, duration_ms: DURATION, at_ms, rate, playing, jump, settling: false }
}

fn playing_from(position_ms: u64, at_ms: u64, rate: f64) -> Playhead {
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
fn inv_playhead_02_a_report_slightly_behind_the_shown_position_does_not_step_back() {
    let shown = playing_from(2000, 1000, 1.0);

    let next = playhead_next(Some(shown), report(2100, 1250, 1.0, true, false));

    near(playhead_at(next, 1250), 0.225);
    near(playhead_at(next, 1750), 0.275);
}

#[test]
fn inv_playhead_03_a_report_far_behind_the_shown_position_snaps_to_the_player() {
    let shown = playing_from(2000, 1000, 1.0);

    let next = playhead_next(Some(shown), report(1900, 1250, 1.0, true, false));

    near(playhead_at(next, 1250), 0.19);
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
