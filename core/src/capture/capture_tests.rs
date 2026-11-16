use super::*;
use crate::api::MediaError;
use crate::ffi::PixelSize;

fn size(width: u32, height: u32) -> PixelSize {
    PixelSize { width, height }
}

fn range(low: u32, high: u32) -> FpsRange {
    FpsRange { low, high }
}

fn photo_profile() -> CaptureProfile {
    capture_profile(CaptureKind::Photo)
}

fn moment() -> CaptureProfile {
    capture_profile(CaptureKind::Moment)
}

#[test]
fn inv_capture_01_the_three_profiles_are_the_table_and_a_bad_profile_is_refused() {
    let photo = photo_profile();
    assert_eq!((photo.mode, photo.photo_size, photo.video_size, photo.fps, photo.max_duration_ms, photo.audio, photo.photo_quality), (CaptureMode::Photo, Some(size(1080, 1920)), None, 30, 0, false, 90));
    let moment = moment();
    assert_eq!((moment.video_size, moment.fps, moment.max_duration_ms, moment.min_take_ms, moment.audio), (Some(size(1280, 720)), 30, 10_000, 500, true));
    let circle = capture_profile(CaptureKind::Circle);
    assert_eq!((circle.video_size, circle.fps, circle.max_duration_ms, circle.min_take_ms, circle.audio), (Some(size(720, 1280)), 30, 60_000, 0, true));
    for kind in [CaptureKind::Photo, CaptureKind::Moment, CaptureKind::Circle] {
        assert_eq!(check_capture_profile(capture_profile(kind)).unwrap(), capture_profile(kind));
    }
    let broken: Vec<(&str, CaptureProfile)> = vec![
        ("a side is zero", CaptureProfile { video_size: Some(size(0, 720)), ..moment }),
        ("a photo side is zero", CaptureProfile { photo_size: Some(size(10, 0)), ..photo }),
        ("fps 0", CaptureProfile { fps: 0, ..moment }),
        ("fps 61", CaptureProfile { fps: 61, ..moment }),
        ("quality 0", CaptureProfile { photo_quality: 0, ..photo }),
        ("quality 101", CaptureProfile { photo_quality: 101, ..photo }),
        ("video without a size", CaptureProfile { video_size: None, ..moment }),
        ("limit under 1 s", CaptureProfile { max_duration_ms: 999, ..moment }),
        ("limit over 10 min", CaptureProfile { max_duration_ms: 600_001, ..moment }),
        ("least take at the limit", CaptureProfile { min_take_ms: 10_000, ..moment }),
        ("photo with a limit", CaptureProfile { max_duration_ms: 5_000, ..photo }),
        ("photo with sound", CaptureProfile { audio: true, ..photo }),
        ("photo with a video size", CaptureProfile { video_size: Some(size(1, 1)), ..photo }),
        ("photo with a stabilization", CaptureProfile { stabilization: VideoStabilization::Standard, ..photo }),
    ];
    for (name, profile) in broken {
        assert!(matches!(check_capture_profile(profile), Err(MediaError::InvalidSpec { .. })), "{name}");
    }
}

#[test]
fn inv_capture_02_the_format_is_picked_by_one_table_of_penalties() {
    let candidate = |video: PixelSize, photos: Vec<PixelSize>, rates: Vec<FpsRange>, high: bool, highest: bool| FormatCandidate { video_size: video, photo_sizes: photos, frame_rates: rates, high_photo_quality: high, highest_photo_quality: highest };
    let sd = candidate(size(640, 480), vec![size(640, 480)], vec![range(15, 30)], false, false);
    let hd = candidate(size(1280, 720), vec![size(4000, 2250)], vec![range(15, 30)], true, true);
    let tall = candidate(size(1080, 1920), vec![size(1080, 1920)], vec![range(24, 60)], true, false);
    let no_photo = candidate(size(1280, 720), vec![], vec![range(30, 30)], false, false);
    let circle = capture_profile(CaptureKind::Circle);
    let pick = |list: &[FormatCandidate], profile| pick_format(list.to_vec(), profile).map(|choice| choice.index);
    let full = candidate(size(1920, 1080), vec![size(1920, 1080)], vec![range(30, 30)], true, false);
    assert_eq!(pick(&[sd.clone(), tall.clone(), hd.clone()], circle), Some(2), "the side order does not matter, the 16:9 area of 720x1280 is the area of 1280x720");
    assert_eq!(pick(&[full, sd.clone(), hd.clone()], moment()), Some(2), "the area nearest to the target wins among formats of one aspect");
    assert_eq!(pick(&[sd.clone(), hd.clone()], moment()), Some(1), "a 4:3 format pays the aspect penalty");
    assert_eq!(pick(&[no_photo.clone(), hd.clone()], photo_profile()), Some(1), "a photo profile skips a format without photo sizes");
    assert_eq!(pick(std::slice::from_ref(&no_photo), photo_profile()), None);
    assert_eq!(pick(&[no_photo], moment()), Some(0), "a video profile does not need photo sizes");
    assert_eq!(pick(&[], moment()), None);
    let choice = pick_format(vec![hd.clone(), tall.clone()], photo_profile()).unwrap();
    assert_eq!((choice.index, choice.photo_size), (1, Some(size(1080, 1920))));
    let rates = candidate(size(1280, 720), vec![], vec![range(60, 120), range(24, 30), range(10, 40)], false, false);
    assert_eq!(pick_format(vec![rates.clone()], moment()).unwrap().frame_rate, range(24, 30), "the range that holds 30 and is the narrowest");
    let slow = candidate(size(1280, 720), vec![], vec![range(10, 24)], false, false);
    assert_eq!(pick_format(vec![slow], moment()).unwrap().frame_rate, range(10, 24), "the nearest range when none holds the target");
    assert_eq!(pick_format(vec![candidate(size(1280, 720), vec![], vec![], false, false)], moment()).unwrap().frame_rate, range(30, 30));
    let ties = [hd.clone(), hd];
    assert_eq!(pick(&ties, moment()), Some(0), "equal penalties keep the first");
}

#[test]
fn inv_capture_03_photo_sizes_follow_the_aspect_of_the_video_and_the_flags_follow_the_pixels() {
    let video = vec![size(1280, 720), size(1920, 1080), size(640, 480)];
    let jpeg = vec![size(4000, 3000), size(4032, 2268), size(1920, 1080), size(1080, 1920), size(640, 480)];
    let list = format_candidates(video, jpeg, vec![range(30, 30)]);
    assert_eq!(list[0].photo_sizes, vec![size(4032, 2268), size(1920, 1080), size(1080, 1920)], "16:9 and 9:16 sit in one aspect, 4:3 does not");
    assert_eq!((list[0].high_photo_quality, list[0].highest_photo_quality), (true, false));
    assert_eq!(list[2].photo_sizes, vec![size(4000, 3000), size(640, 480)]);
    assert_eq!((list[2].high_photo_quality, list[2].highest_photo_quality), (true, true));
    assert!(list.iter().all(|candidate| candidate.frame_rates == vec![range(30, 30)]));
    let none = format_candidates(vec![size(1280, 720)], vec![], vec![]);
    assert_eq!((none[0].photo_sizes.len(), none[0].high_photo_quality, none[0].highest_photo_quality), (0, false, false));
}

#[test]
fn inv_capture_04_the_zoom_stays_between_the_minimum_and_the_ceiling() {
    for (current, factor, min, max, expected) in [
        (1.0, 2.0, 1.0, 10.0, 2.0),
        (4.0, 10.0, 1.0, 30.0, 15.0),
        (4.0, 0.1, 1.0, 10.0, 1.0),
        (2.0, 1.5, 1.0, 2.5, 2.5),
        (2.0, f32::NAN, 1.0, 10.0, 2.0),
        (2.0, 0.0, 1.0, 10.0, 2.0),
        (2.0, -3.0, 1.0, 10.0, 2.0),
        (2.0, f32::INFINITY, 1.0, 10.0, 2.0),
        (1.0, 2.0, 3.0, 2.0, 3.0),
    ] {
        assert_eq!(zoom_target(current, factor, min, max), expected, "{current} * {factor} in {min}..{max}");
    }
}

#[test]
fn inv_capture_06_the_verdict_of_a_recording_follows_the_table() {
    use RecordEnd::{Completed, Failed, LimitReached};
    use RecordVerdict as V;
    for (started, closed, discard, end, length, expected) in [
        (true, true, false, Completed, Some(900), V::Skip),
        (true, false, true, Completed, Some(900), V::Skip),
        (false, true, false, Failed, None, V::Skip),
        (false, false, false, Completed, Some(900), V::CameraNotReady),
        (true, false, false, Failed, Some(900), V::Failed),
        (true, false, false, Completed, None, V::Failed),
        (true, false, false, Completed, Some(500), V::TooShort),
        (true, false, false, Completed, Some(0), V::TooShort),
        (true, false, false, Completed, Some(501), V::Deliver),
        (true, false, false, LimitReached, Some(10_000), V::Deliver),
    ] {
        assert_eq!(record_verdict(started, closed, discard, end, length, 500), expected, "{started} {closed} {discard} {end:?} {length:?}");
    }
    assert_eq!(record_verdict(true, false, false, Completed, Some(1), 0), V::Deliver);
    assert_eq!(record_verdict(true, false, false, Completed, Some(0), 0), V::TooShort);
}

#[test]
fn inv_capture_08_the_frame_gate_passes_the_first_frame_and_then_the_rate() {
    assert!(frame_gate(None, 5, 12.0));
    assert!(!frame_gate(Some(1000), 1050, 12.0));
    assert!(!frame_gate(Some(1000), 1083, 12.0));
    assert!(frame_gate(Some(1000), 1084, 12.0));
    assert!(frame_gate(Some(1000), 900, 12.0), "a clock that went back resets the mark");
    assert!(!frame_gate(Some(1000), 5000, 0.0));
    assert!(!frame_gate(Some(1000), 5000, f64::NAN));
    assert_eq!(capture_tick_ms(), 100);
}

fn drive(profile: CaptureProfile, run: CaptureRun, events: &[CaptureEvent]) -> (CaptureRun, Vec<CaptureAction>) {
    let mut run = run;
    let mut actions = Vec::new();
    for event in events {
        let step = capture_step(run, *event, profile);
        run = step.run;
        actions.extend(step.actions);
    }
    (run, actions)
}

fn ready(profile: CaptureProfile) -> CaptureRun {
    let open = [CaptureEvent::DemandOn, CaptureEvent::Opened { has_flash: true, zoom_min: 1.0, zoom_max: 30.0 }, CaptureEvent::Streaming { on: true }];
    drive(profile, capture_run(CameraLens::Back), &open).0
}

fn recording(profile: CaptureProfile) -> CaptureRun {
    drive(profile, ready(profile), &[CaptureEvent::Start, CaptureEvent::RecordStarted]).0
}

fn phase(run: &CaptureRun) -> CameraPhase {
    run.state.phase
}

#[test]
fn inv_capture_09_a_command_acts_only_in_the_phase_that_takes_it() {
    let photo = photo_profile();
    let moment = moment();
    let idle = capture_run(CameraLens::Back);
    let opening = drive(photo, idle, &[CaptureEvent::DemandOn]).0;
    for (name, profile, run, event) in [
        ("photo in idle", photo, idle, CaptureEvent::Photo),
        ("photo while opening", photo, opening, CaptureEvent::Photo),
        ("start in idle", moment, idle, CaptureEvent::Start),
        ("stop in ready", moment, ready(moment), CaptureEvent::Stop),
        ("photo with a video profile", moment, ready(moment), CaptureEvent::Photo),
        ("start with a photo profile", photo, ready(photo), CaptureEvent::Start),
        ("start while recording", moment, recording(moment), CaptureEvent::Start),
        ("photo while recording", moment, recording(moment), CaptureEvent::Photo),
        ("zoom while opening", photo, opening, CaptureEvent::ZoomBy { factor: 2.0 }),
        ("cancel in ready", moment, ready(moment), CaptureEvent::Cancel),
    ] {
        let step = capture_step(run, event, profile);
        assert_eq!((step.run, step.actions), (run, vec![]), "{name}");
    }
    let shot = drive(photo, ready(photo), &[CaptureEvent::Photo]);
    assert_eq!(shot.1, vec![CaptureAction::TakePhoto]);
    assert!(shot.0.state.shooting);
    let again = drive(photo, shot.0, &[CaptureEvent::Photo]);
    assert_eq!((again.0, again.1.len()), (shot.0, 0), "a second shot waits for the first photo");
    let done = drive(photo, shot.0, &[CaptureEvent::PhotoDone { ok: false }]).0;
    assert_eq!((done.state.shooting, done.state.fault, phase(&done)), (false, Some(CameraRefusal::PhotoFailed), CameraPhase::Ready));
    let good = drive(photo, done, &[CaptureEvent::Photo, CaptureEvent::PhotoDone { ok: true }]).0;
    assert_eq!(good.state.fault, None);
}

#[test]
fn inv_capture_10_a_lens_change_during_a_recording_throws_it_away_and_switches_after_its_end() {
    let profile = moment();
    let (run, actions) = drive(profile, recording(profile), &[CaptureEvent::SetLens { lens: CameraLens::Front }]);
    assert_eq!((phase(&run), actions), (CameraPhase::Stopping, vec![CaptureAction::EndRecording { discard: true }]));
    assert_eq!(run.state.lens, CameraLens::Back, "the lens stays until the recording ended");
    let (after, actions) = drive(profile, run, &[CaptureEvent::RecordFinished { end: RecordEnd::Completed }]);
    assert_eq!(actions, vec![CaptureAction::Discard, CaptureAction::Bind { lens: CameraLens::Front }]);
    assert_eq!((phase(&after), after.state.lens, after.state.fault), (CameraPhase::Opening, CameraLens::Front, None));

    let photo = photo_profile();
    let mut flashed = ready(photo);
    flashed = drive(photo, flashed, &[CaptureEvent::SetFlash { on: true }]).0;
    assert!(flashed.state.flash);
    let (flipped, actions) = drive(photo, flashed, &[CaptureEvent::SetLens { lens: CameraLens::Front }]);
    assert_eq!((phase(&flipped), flipped.state.flash, actions), (CameraPhase::Opening, false, vec![CaptureAction::Bind { lens: CameraLens::Front }]));

    let mut torch = ready(profile);
    torch = drive(profile, torch, &[CaptureEvent::SetFlash { on: true }]).0;
    let flipped = drive(profile, torch, &[CaptureEvent::SetLens { lens: CameraLens::Front }]).0;
    assert!(flipped.state.flash, "the torch of a video stays on a lens change");
    let (reopened, actions) = drive(profile, flipped, &[CaptureEvent::Opened { has_flash: true, zoom_min: 1.0, zoom_max: 8.0 }]);
    assert_eq!(actions, vec![CaptureAction::SetTorch { on: true }]);
    assert_eq!(reopened.state.zoom_max, 8.0);
    let (no_unit, actions) = drive(profile, flipped, &[CaptureEvent::Opened { has_flash: false, zoom_min: 1.0, zoom_max: 8.0 }]);
    assert_eq!((no_unit.state.flash, actions.len()), (false, 0), "a camera without a flash unit drops the torch");
}

#[test]
fn inv_capture_11_leaving_the_screen_unbinds_in_every_phase_and_throws_a_recording_away() {
    let profile = moment();
    let idle = capture_run(CameraLens::Front);
    let runs = [
        ("opening", drive(profile, idle, &[CaptureEvent::DemandOn]).0),
        ("ready", ready(profile)),
        ("recording", recording(profile)),
        ("stopping", drive(profile, recording(profile), &[CaptureEvent::Stop]).0),
        ("failed", drive(profile, idle, &[CaptureEvent::DemandOn, CaptureEvent::Refuse { reason: CameraRefusal::NoCamera }]).0),
    ];
    for (name, run) in runs {
        let (after, actions) = drive(profile, run, &[CaptureEvent::DemandOff]);
        assert_eq!(actions, vec![CaptureAction::Unbind], "{name}");
        assert_eq!((phase(&after), after.state.recorded_ms, after.state.fault, after.starting, after.started, after.open), (CameraPhase::Idle, 0, None, false, false, false), "{name}");
        let late = drive(profile, after, &[CaptureEvent::RecordFinished { end: RecordEnd::Completed }, CaptureEvent::RecordSettled { duration_ms: Some(9000) }, CaptureEvent::Streaming { on: true }]);
        assert_eq!((late.0, late.1.len()), (after, 0), "{name}: late platform events do nothing");
    }
    let (again, actions) = drive(profile, capture_run(CameraLens::Back), &[CaptureEvent::DemandOff]);
    assert_eq!((phase(&again), actions.len()), (CameraPhase::Idle, 0));
    let (back, actions) = drive(profile, runs_lens_front(), &[CaptureEvent::DemandOff, CaptureEvent::DemandOn]);
    assert_eq!((back.state.lens, actions.last().copied()), (CameraLens::Front, Some(CaptureAction::Bind { lens: CameraLens::Front })), "the lens survives a leave");
}

fn runs_lens_front() -> CaptureRun {
    drive(photo_profile(), capture_run(CameraLens::Back), &[CaptureEvent::SetLens { lens: CameraLens::Front }, CaptureEvent::DemandOn]).0
}

#[test]
fn inv_capture_12_ready_needs_an_open_camera_and_a_running_preview() {
    let profile = photo_profile();
    let open = CaptureEvent::Opened { has_flash: false, zoom_min: 1.0, zoom_max: 5.0 };
    let phases = |events: &[CaptureEvent]| phase(&drive(profile, capture_run(CameraLens::Back), events).0);
    assert_eq!(phases(&[CaptureEvent::DemandOn, open]), CameraPhase::Opening);
    assert_eq!(phases(&[CaptureEvent::DemandOn, CaptureEvent::Streaming { on: true }]), CameraPhase::Opening);
    assert_eq!(phases(&[CaptureEvent::DemandOn, CaptureEvent::Streaming { on: true }, open]), CameraPhase::Ready);
    assert_eq!(phases(&[CaptureEvent::DemandOn, open, CaptureEvent::Streaming { on: true }]), CameraPhase::Ready);
    assert_eq!(phases(&[CaptureEvent::DemandOn, open, CaptureEvent::Streaming { on: true }, CaptureEvent::Streaming { on: false }]), CameraPhase::Opening);
    assert_eq!(phases(&[CaptureEvent::DemandOn, open, CaptureEvent::Streaming { on: true }, CaptureEvent::CameraClosed]), CameraPhase::Opening);
    assert_eq!(phases(&[CaptureEvent::DemandOn, open, CaptureEvent::Streaming { on: true }, CaptureEvent::CameraClosed, open]), CameraPhase::Ready);
    let moment = moment();
    let during = drive(moment, recording(moment), &[CaptureEvent::Streaming { on: false }]).0;
    assert_eq!(phase(&during), CameraPhase::Recording, "a recording does not stop because the preview stalls");
}

#[test]
fn inv_capture_19_a_reset_restarts_once_per_demand_and_the_second_one_fails() {
    let profile = moment();
    let reset = CaptureEvent::Fault { kind: FaultKind::Reset };
    let (restarted, actions) = drive(profile, recording(profile), &[reset]);
    assert_eq!(actions, vec![CaptureAction::Restart { lens: CameraLens::Back }]);
    assert_eq!((phase(&restarted), restarted.recovery_spent, restarted.started, restarted.state.recorded_ms), (CameraPhase::Opening, true, false, 0));
    let (failed, actions) = drive(profile, restarted, &[reset]);
    assert_eq!((phase(&failed), failed.state.fault, actions), (CameraPhase::Failed, Some(CameraRefusal::Busy), vec![CaptureAction::Unbind]));
    let (other, _) = drive(profile, ready(profile), &[CaptureEvent::Fault { kind: FaultKind::Other }]);
    assert_eq!((phase(&other), other.state.fault), (CameraPhase::Failed, Some(CameraRefusal::ConfigureFailed)));
    let (again, actions) = drive(profile, failed, &[CaptureEvent::DemandOn]);
    assert_eq!((phase(&again), again.recovery_spent, again.state.fault, actions), (CameraPhase::Opening, false, None, vec![CaptureAction::Bind { lens: CameraLens::Back }]));
    let idle_fault = drive(profile, capture_run(CameraLens::Back), &[reset]);
    assert_eq!((idle_fault.0, idle_fault.1.len()), (capture_run(CameraLens::Back), 0));
}

#[test]
fn inv_capture_20_a_recording_runs_start_to_verdict_and_the_limit_ends_it_by_itself() {
    let profile = moment();
    let (starting, actions) = drive(profile, ready(profile), &[CaptureEvent::Start]);
    assert_eq!((phase(&starting), starting.starting, actions), (CameraPhase::Ready, true, vec![CaptureAction::BeginRecording]));
    assert!(drive(profile, starting, &[CaptureEvent::Start]).1.is_empty(), "no second start before the start event");
    let running = drive(profile, starting, &[CaptureEvent::RecordStarted, CaptureEvent::Tick { recorded_ms: 300 }]).0;
    assert_eq!((phase(&running), running.state.recorded_ms), (CameraPhase::Recording, 300));
    assert_eq!(drive(profile, ready(profile), &[CaptureEvent::Tick { recorded_ms: 9 }]).0.state.recorded_ms, 0, "no length outside a recording");

    let stopped = drive(profile, running, &[CaptureEvent::Stop, CaptureEvent::RecordFinished { end: RecordEnd::Completed }]);
    assert_eq!((phase(&stopped.0), stopped.1.clone()), (CameraPhase::Stopping, vec![CaptureAction::EndRecording { discard: false }, CaptureAction::ReadClip]));
    let delivered = drive(profile, stopped.0, &[CaptureEvent::RecordSettled { duration_ms: Some(2_000) }]);
    assert_eq!((delivered.1, phase(&delivered.0), delivered.0.state.fault, delivered.0.state.recorded_ms), (vec![CaptureAction::Deliver], CameraPhase::Ready, None, 0));

    let by_limit = drive(profile, running, &[CaptureEvent::RecordFinished { end: RecordEnd::LimitReached }, CaptureEvent::RecordSettled { duration_ms: Some(10_000) }]);
    assert_eq!((by_limit.1, phase(&by_limit.0)), (vec![CaptureAction::ReadClip, CaptureAction::Deliver], CameraPhase::Ready));

    for (name, end, length, action, fault) in [
        ("too short", RecordEnd::Completed, Some(400), CaptureAction::Discard, Some(CameraRefusal::TooShort)),
        ("no length", RecordEnd::Completed, None, CaptureAction::Discard, Some(CameraRefusal::RecordFailed)),
        ("failed end", RecordEnd::Failed, Some(2_000), CaptureAction::Discard, Some(CameraRefusal::RecordFailed)),
    ] {
        let (after, actions) = drive(profile, running, &[CaptureEvent::RecordFinished { end }, CaptureEvent::RecordSettled { duration_ms: length }]);
        assert_eq!((actions.last().copied(), after.state.fault, phase(&after)), (Some(action), fault, CameraPhase::Ready), "{name}");
    }
    let (never, actions) = drive(profile, starting, &[CaptureEvent::RecordFinished { end: RecordEnd::Failed }]);
    assert_eq!((actions, never.state.fault, phase(&never)), (vec![CaptureAction::Discard], Some(CameraRefusal::NotReady), CameraPhase::Ready));

    let (cancelled, actions) = drive(profile, running, &[CaptureEvent::Cancel, CaptureEvent::RecordFinished { end: RecordEnd::Completed }]);
    assert_eq!((actions, cancelled.state.fault, phase(&cancelled)), (vec![CaptureAction::EndRecording { discard: true }, CaptureAction::Discard], None, CameraPhase::Ready));
    let (early, actions) = drive(profile, starting, &[CaptureEvent::Cancel, CaptureEvent::RecordStarted, CaptureEvent::RecordFinished { end: RecordEnd::Completed }]);
    assert_eq!((actions, phase(&early)), (vec![CaptureAction::EndRecording { discard: true }, CaptureAction::Discard], CameraPhase::Ready), "a cancel before the start event never records");
}

#[test]
fn inv_capture_05_the_flash_and_the_zoom_of_a_camera_follow_its_unit_and_its_range() {
    let profile = moment();
    let (on, actions) = drive(profile, ready(profile), &[CaptureEvent::SetFlash { on: true }, CaptureEvent::SetFlash { on: true }]);
    assert_eq!((on.state.flash, actions), (true, vec![CaptureAction::SetTorch { on: true }]));
    let open = [CaptureEvent::DemandOn, CaptureEvent::Opened { has_flash: false, zoom_min: 1.0, zoom_max: 4.0 }, CaptureEvent::Streaming { on: true }, CaptureEvent::SetFlash { on: true }];
    let (without, actions) = drive(profile, capture_run(CameraLens::Back), &open);
    assert_eq!((without.state.flash, actions.len()), (false, 1), "the Bind is the only action: no unit, no torch");
    let (zoomed, actions) = drive(profile, ready(profile), &[CaptureEvent::ZoomBy { factor: 2.0 }, CaptureEvent::ZoomBy { factor: 100.0 }, CaptureEvent::ZoomBy { factor: 100.0 }]);
    assert_eq!((zoomed.state.zoom, actions), (15.0, vec![CaptureAction::SetZoom { ratio: 2.0 }, CaptureAction::SetZoom { ratio: 15.0 }]));
    let wide = drive(profile, capture_run(CameraLens::Back), &[CaptureEvent::DemandOn, CaptureEvent::Opened { has_flash: false, zoom_min: 2.0, zoom_max: 6.0 }]).0;
    assert_eq!((wide.state.zoom, wide.state.zoom_min, wide.state.zoom_max), (2.0, 2.0, 6.0), "the start zoom is inside the range");
}

#[test]
fn inv_capture_14_capture_files_have_one_name_scheme_and_only_those_names_are_orphans() {
    assert_eq!(capture_file_name(CaptureFileKind::Photo, "a1".to_string()), "capture-a1.jpg");
    assert_eq!(capture_file_name(CaptureFileKind::Clip, "a1".to_string()), "capture-a1.mp4");
    for (name, expected) in [("capture-a1.jpg", true), ("capture-a1.mp4", true), ("capture-.jpg", true), ("voice-a1.m4a", false), ("capture-a1.mov", false), ("moment-a1.mp4", false), ("capture-a1", false)] {
        assert_eq!(is_capture_orphan(name.to_string()), expected, "{name}");
    }
}

#[test]
fn inv_capture_13_the_costs_are_rasters_of_the_session_and_of_the_shot() {
    let px = |width: u64, height: u64| width * height * 4;
    assert_eq!(capture_cost(moment()), px(1280, 720) * 3);
    assert_eq!(capture_cost(capture_profile(CaptureKind::Circle)), px(720, 1280) * 3);
    assert_eq!(capture_cost(photo_profile()), px(1080, 1920) * 2);
    assert_eq!(shot_cost(4000, 3000), Some(px(4000, 3000) * 2));
    assert_eq!((shot_cost(0, 10), shot_cost(16_385, 1)), (None, None));
    for reason in [CameraRefusal::PermissionDenied, CameraRefusal::NoCamera, CameraRefusal::Busy, CameraRefusal::MicrophoneBusy, CameraRefusal::ConfigureFailed, CameraRefusal::NotReady, CameraRefusal::TooShort, CameraRefusal::RecordFailed, CameraRefusal::PhotoFailed, CameraRefusal::Memory, CameraRefusal::InCall] {
        assert!(!camera_refusal_text(reason).is_empty());
    }
}

#[test]
fn inv_capture_26_the_stabilization_is_set_only_when_the_device_offers_it() {
    use VideoStabilization::{Cinematic, Off, Standard};
    for (requested, supported, expected) in [
        (Cinematic, vec![Standard, Cinematic], Some(Cinematic)),
        (Cinematic, vec![Standard], None),
        (Standard, vec![], None),
        (Off, vec![], Some(Off)),
    ] {
        assert_eq!(stabilization_mode(requested, supported.clone()), expected, "{requested:?} in {supported:?}");
    }
}

#[test]
fn inv_capture_28_a_too_short_recording_keeps_its_length_in_the_state() {
    let profile = moment();
    let recording = drive(profile, ready(profile), &[CaptureEvent::Start, CaptureEvent::RecordStarted]).0;
    for (name, end, length, refused) in [
        ("under the least take", RecordEnd::Completed, Some(400), Some(400)),
        ("at the least take", RecordEnd::Completed, Some(500), Some(500)),
        ("zero length", RecordEnd::Completed, Some(0), Some(0)),
        ("above the least take", RecordEnd::Completed, Some(501), None),
        ("by the limit", RecordEnd::LimitReached, Some(10_000), None),
        ("no length", RecordEnd::Completed, None, None),
        ("failed end", RecordEnd::Failed, Some(400), None),
    ] {
        let (after, _) = drive(profile, recording, &[CaptureEvent::RecordFinished { end }, CaptureEvent::RecordSettled { duration_ms: length }]);
        assert_eq!((after.state.refused_ms, after.state.recorded_ms), (refused, 0), "{name}");
    }
    let (short, _) = drive(profile, recording, &[CaptureEvent::RecordFinished { end: RecordEnd::Completed }, CaptureEvent::RecordSettled { duration_ms: Some(400) }]);
    assert_eq!(drive(profile, short, &[CaptureEvent::Start]).0.state.refused_ms, None, "a new recording starts without the old length");
    let (cancelled, _) = drive(profile, recording, &[CaptureEvent::Cancel, CaptureEvent::RecordFinished { end: RecordEnd::Completed }]);
    assert_eq!(cancelled.state.refused_ms, None, "a thrown away recording refuses nothing");
}

#[test]
fn inv_call_08_a_call_throws_a_running_recording_away_and_ends_it_with_the_in_call_refusal() {
    let profile = moment();
    let recording = drive(profile, ready(profile), &[CaptureEvent::Start, CaptureEvent::RecordStarted]).0;

    let (ended, actions) = drive(profile, recording, &[CaptureEvent::Revoked { reason: CameraRefusal::InCall }, CaptureEvent::RecordFinished { end: RecordEnd::Completed }]);
    assert_eq!((actions, ended.state.fault, phase(&ended)), (vec![CaptureAction::EndRecording { discard: true }, CaptureAction::Discard], Some(CameraRefusal::InCall), CameraPhase::Ready));

    let idle = ready(profile);
    let (untouched, actions) = drive(profile, idle, &[CaptureEvent::Revoked { reason: CameraRefusal::InCall }]);
    assert_eq!((actions, untouched.state.fault, phase(&untouched)), (Vec::new(), None, CameraPhase::Ready), "a call without a recording changes nothing");
}

#[test]
fn inv_capture_30_a_lens_change_in_a_passing_phase_is_applied_when_that_phase_settles() {
    let profile = moment();
    let front = CaptureEvent::SetLens { lens: CameraLens::Front };
    let back = CaptureEvent::SetLens { lens: CameraLens::Back };
    let finished = CaptureEvent::RecordFinished { end: RecordEnd::Completed };

    let (cancelled, actions) = drive(profile, recording(profile), &[CaptureEvent::Cancel, front]);
    assert_eq!((actions, cancelled.state.lens), (vec![CaptureAction::EndRecording { discard: true }], CameraLens::Back), "the lens waits for the cancel to settle");
    let (after, actions) = drive(profile, cancelled, &[finished]);
    assert_eq!((actions, after.state.lens, phase(&after)), (vec![CaptureAction::Discard, CaptureAction::Bind { lens: CameraLens::Front }], CameraLens::Front, CameraPhase::Opening), "cancel then lens");

    let (stopped, _) = drive(profile, recording(profile), &[CaptureEvent::Stop, front, finished]);
    let (after, actions) = drive(profile, stopped, &[CaptureEvent::RecordSettled { duration_ms: Some(2_000) }]);
    assert_eq!((actions, after.state.lens), (vec![CaptureAction::Deliver, CaptureAction::Bind { lens: CameraLens::Front }], CameraLens::Front), "stop then lens keeps the clip and switches");

    let (after, actions) = drive(profile, cancelled, &[back, finished]);
    assert_eq!((actions, after.state.lens), (vec![CaptureAction::Discard], CameraLens::Back), "a lens asked back to the current one switches nothing");

    let starting = drive(profile, ready(profile), &[CaptureEvent::Start]).0;
    let (after, actions) = drive(profile, starting, &[front, CaptureEvent::RecordStarted, finished]);
    assert_eq!((actions.last().copied(), after.state.lens), (Some(CaptureAction::Bind { lens: CameraLens::Front }), CameraLens::Front), "a lens asked before the start event throws the take away and switches");
    assert!(actions.contains(&CaptureAction::Discard), "{actions:?}");

    let photo = photo_profile();
    let shooting = drive(photo, ready(photo), &[CaptureEvent::Photo]).0;
    let (waiting, actions) = drive(photo, shooting, &[front]);
    assert_eq!((actions.len(), waiting.state.lens), (0, CameraLens::Back), "the lens waits for the photo");
    let (after, actions) = drive(photo, waiting, &[CaptureEvent::PhotoDone { ok: true }]);
    assert_eq!((actions, after.state.lens), (vec![CaptureAction::Bind { lens: CameraLens::Front }], CameraLens::Front), "photo then lens");
}

#[test]
fn inv_capture_32_a_take_starts_only_while_its_microphone_sound_flows_and_losing_it_throws_the_take_away() {
    let profile = moment();
    let silent = drive(profile, ready(profile), &[CaptureEvent::AudioFlow { flowing: false }]).0;
    let (refused, actions) = drive(profile, silent, &[CaptureEvent::Start]);
    assert_eq!((actions, refused.state.fault, phase(&refused)), (Vec::new(), Some(CameraRefusal::MicrophoneSilent), CameraPhase::Ready));

    let flowing = drive(profile, refused, &[CaptureEvent::AudioFlow { flowing: true }]).0;
    let (started, actions) = drive(profile, flowing, &[CaptureEvent::Start, CaptureEvent::RecordStarted]);
    assert_eq!((actions, phase(&started)), (vec![CaptureAction::BeginRecording], CameraPhase::Recording));

    let (ended, actions) = drive(profile, started, &[CaptureEvent::AudioFlow { flowing: false }, CaptureEvent::RecordFinished { end: RecordEnd::Completed }]);
    assert_eq!((actions, ended.state.fault, phase(&ended)), (vec![CaptureAction::EndRecording { discard: true }, CaptureAction::Discard], Some(CameraRefusal::MicrophoneSilent), CameraPhase::Ready));

    let quiet = CaptureProfile { audio: false, ..moment() };
    let muted = drive(quiet, ready(quiet), &[CaptureEvent::AudioFlow { flowing: false }]).0;
    assert_eq!(drive(quiet, muted, &[CaptureEvent::Start]).1, vec![CaptureAction::BeginRecording], "a profile without sound does not wait for the microphone");
}

#[test]
fn inv_capture_33_the_silence_of_a_take_ends_with_it_and_the_next_start_follows_the_sensor() {
    let profile = moment();
    let silent_take = || {
        let flowing = drive(profile, ready(profile), &[CaptureEvent::AudioFlow { flowing: true }]).0;
        drive(profile, flowing, &[CaptureEvent::Start, CaptureEvent::RecordStarted, CaptureEvent::AudioFlow { flowing: false }, CaptureEvent::RecordFinished { end: RecordEnd::Completed }]).0
    };
    let (_, actions) = drive(profile, silent_take(), &[CaptureEvent::Start]);
    assert_eq!(actions, vec![CaptureAction::BeginRecording], "a sensor that reports only during a take (CameraX AudioStats) does not lock the camera after a silent take");

    let (still, actions) = drive(profile, silent_take(), &[CaptureEvent::AudioFlow { flowing: false }, CaptureEvent::Start]);
    assert_eq!((actions, still.state.fault), (Vec::new(), Some(CameraRefusal::MicrophoneSilent)), "a sensor that still hears silence after the take refuses the next start");
}
