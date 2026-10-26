use super::*;
use crate::api::MediaError;

fn source(width: u32, height: u32) -> VideoSource {
    VideoSource { width, height, rotation_degrees: 0, frame_rate: 30.0, bitrate: Some(8_000_000), mp4_h264_aac: true }
}

fn plan(source: VideoSource) -> VideoPlan {
    plan_video(source, default_video_limits()).unwrap()
}

#[test]
fn inv_send_01_the_sides_of_the_plan_are_even_and_never_below_two() {
    for (width, height, expected) in [(1281, 721, (1280, 720)), (3, 5, (2, 4)), (1, 1, (2, 2)), (1919, 1079, (1918, 1078))] {
        let plan = plan(source(width, height));
        assert_eq!((plan.width, plan.height), expected, "{width}x{height}");
    }
    let cut = plan(source(4001, 3001));
    assert_eq!((cut.width % 2, cut.height % 2), (0, 0));
}

#[test]
fn inv_send_02_the_long_side_follows_the_table_and_a_small_video_is_not_scaled_up() {
    assert_eq!((plan(source(3840, 2160)).width, plan(source(3840, 2160)).height), (1920, 1080));
    assert_eq!((plan(source(2160, 3840)).width, plan(source(2160, 3840)).height), (1080, 1920));
    assert_eq!((plan(source(640, 360)).width, plan(source(640, 360)).height), (640, 360));
    let mut limits = default_video_limits();
    limits.max_long_side = 48;
    let small = plan_video(source(32, 64), limits).unwrap();
    assert_eq!((small.width, small.height), (24, 48));
}

#[test]
fn inv_send_03_a_quarter_turn_swaps_the_sides_and_the_plan_names_the_turn() {
    let mut turned = source(1920, 1080);
    for (degrees, sides, turn) in [(90, (1080, 1920), 90), (270, (1080, 1920), 270), (-90, (1080, 1920), 270), (180, (1920, 1080), 180), (360, (1920, 1080), 0)] {
        turned.rotation_degrees = degrees;
        let plan = plan(turned);
        assert_eq!(((plan.width, plan.height), plan.rotation_degrees), (sides, turn), "{degrees} degrees");
    }
}

#[test]
fn inv_send_04_the_bitrate_stays_in_the_corridor_of_the_target_and_below_the_source_cap() {
    let mut known = source(1920, 1080);
    known.bitrate = None;
    assert_eq!(plan(known).bitrate, 2_000_000);
    known.bitrate = Some(50_000_000);
    assert_eq!(plan(known).bitrate, 3_500_000);
    known.bitrate = Some(100_000);
    assert_eq!(plan(known).bitrate, 2_000_000);
    known.bitrate = Some(3_000_000);
    assert_eq!(plan(known).bitrate, 2_850_000);
    let mut small = source(640, 360);
    small.bitrate = Some(10_000_000);
    assert_eq!(plan(small).bitrate, 900_000);
    let mut reduced = source(3840, 2160);
    reduced.bitrate = Some(20_000_000);
    assert_eq!(plan(reduced).bitrate, 3_500_000);
}

#[test]
fn inv_send_05_the_frame_rate_is_rounded_limited_and_defaults_when_unknown() {
    let mut rated = source(1280, 720);
    for (rate, expected) in [(29.97, 30), (24.0, 24), (120.0, 60), (0.0, 30), (-5.0, 30), (0.4, 30)] {
        rated.frame_rate = rate;
        assert_eq!(plan(rated).frame_rate, expected, "{rate} fps");
    }
}

#[test]
fn inv_send_06_a_file_that_already_fits_is_handed_back_and_any_misfit_is_encoded() {
    let mut fits = source(1280, 720);
    fits.bitrate = Some(1_500_000);
    assert!(!plan(fits).reencode);
    type Misfit = (&'static str, fn(&mut VideoSource));
    let misfits: [Misfit; 7] = [
        ("high bitrate", |source| source.bitrate = Some(5_000_000)),
        ("unknown bitrate", |source| source.bitrate = None),
        ("turned", |source| source.rotation_degrees = 90),
        ("other codec", |source| source.mp4_h264_aac = false),
        ("fast", |source| source.frame_rate = 120.0),
        ("unknown rate", |source| source.frame_rate = 0.0),
        ("large", |source| source.width = 2560),
    ];
    for (name, change) in misfits {
        let mut misfit = fits;
        change(&mut misfit);
        assert!(plan(misfit).reencode, "{name}");
    }
}

#[test]
fn inv_send_07_limits_the_planner_cannot_work_with_are_refused() {
    let broken: [fn(&mut VideoLimits); 4] = [
        |limits| limits.max_long_side = 0,
        |limits| limits.corridors.clear(),
        |limits| limits.corridors[0].min_bps = limits.corridors[0].max_bps + 1,
        |limits| limits.min_frame_rate = limits.max_frame_rate + 1,
    ];
    for change in broken {
        let mut limits = default_video_limits();
        change(&mut limits);
        assert!(matches!(plan_video(source(1280, 720), limits), Err(MediaError::InvalidSpec { .. })));
    }
    assert!(matches!(plan_video(source(0, 720), default_video_limits()), Err(MediaError::InvalidSpec { .. })));
}

#[test]
fn inv_send_08_the_eight_exif_orientations_make_a_photo_upright() {
    let turn = |rotation_degrees, flip_horizontal| PhotoTurn { rotation_degrees, flip_horizontal };
    let table = [(1, turn(0, false)), (2, turn(0, true)), (3, turn(180, false)), (4, turn(180, true)), (5, turn(90, true)), (6, turn(90, false)), (7, turn(270, true)), (8, turn(270, false))];
    for (tag, expected) in table {
        assert_eq!(photo_turn(tag), expected, "tag {tag}");
    }
    for tag in [0, 9, 99] {
        assert_eq!(photo_turn(tag), turn(0, false), "tag {tag}");
    }
}

#[test]
fn inv_send_09_the_jpeg_quality_stays_between_zero_and_one() {
    for (given, expected) in [(0.5, 0.5), (-1.0, 0.0), (7.0, 1.0), (f64::NAN, 0.9), (f64::INFINITY, 1.0)] {
        assert_eq!(jpeg_quality(given), expected);
    }
}

#[test]
fn inv_image_70_a_turned_picture_swaps_its_sides_and_pays_for_two_rasters() {
    let upright = image_decode_plan(400, 200, 1, None).unwrap();
    assert_eq!((upright.width, upright.height, upright.cost), (400, 200, 400 * 200 * 4));
    for (tag, degrees) in [(6, 90), (8, 270)] {
        let plan = image_decode_plan(400, 200, tag, None).unwrap();
        assert_eq!((plan.decode_width, plan.decode_height), (400, 200), "tag {tag}");
        assert_eq!((plan.width, plan.height, plan.turn.rotation_degrees), (200, 400, degrees), "tag {tag}");
        assert_eq!(plan.cost, 400 * 200 * 4 * 2, "tag {tag}");
    }
}

#[test]
fn inv_image_71_the_long_side_is_cut_to_the_limit_and_a_small_picture_is_not_enlarged() {
    let cut = image_decode_plan(4000, 3000, 6, Some(1000)).unwrap();
    assert_eq!((cut.decode_width, cut.decode_height, cut.width, cut.height), (1000, 750, 750, 1000));
    assert_eq!(cut.cost, 1000 * 750 * 4 * 2);
    for limit in [Some(5000), Some(0), None] {
        let whole = image_decode_plan(4000, 3000, 1, limit).unwrap();
        assert_eq!((whole.decode_width, whole.decode_height), (4000, 3000), "limit {limit:?}");
    }
    assert_eq!(image_decode_plan(1, 4000, 1, Some(10)).unwrap().decode_width, 1);
}

#[test]
fn inv_image_72_a_picture_without_size_or_above_the_pixel_limit_has_no_plan_unless_the_limit_cuts_it() {
    assert_eq!(image_decode_plan(0, 10, 1, None), None);
    assert_eq!(image_decode_plan(16_385, 10, 1, None), None);
    assert!(image_decode_plan(16_385, 10, 1, Some(2048)).is_some());
}

#[test]
fn inv_voice_01_the_level_is_in_zero_to_one_for_any_peak_and_follows_the_decibel_scale() {
    for (peak, expected) in [(1.0, 1.0), (2.0, 1.0), (0.0, 0.0), (-1.0, 0.0), (f32::NAN, 0.0), (0.001, 0.0), (0.0316228, 0.4)] {
        let level = voice_level(peak);
        assert!((0.0..=1.0).contains(&level), "{peak}");
        assert!((level - expected).abs() < 0.001, "{peak} gave {level}");
    }
    assert!(voice_level(0.1) > voice_level(0.05));
}

#[test]
fn inv_voice_02_the_recording_limit_has_a_default_and_stays_between_one_second_and_one_hour() {
    for (requested, expected) in [(None, 600_000), (Some(0), 1_000), (Some(30_000), 30_000), (Some(u64::MAX), 3_600_000)] {
        assert_eq!(voice_limit_ms(requested), expected, "{requested:?}");
    }
}

#[test]
fn inv_voice_03_every_refusal_has_its_own_text() {
    let texts = [VoiceRefusal::PermissionDenied, VoiceRefusal::MicrophoneBusy, VoiceRefusal::Empty].map(voice_refusal_text);
    assert!(texts.iter().all(|text| !text.is_empty()));
    assert_eq!(texts.iter().collect::<std::collections::HashSet<_>>().len(), 3);
}

#[test]
fn inv_voice_12_a_voice_recording_is_encoded_at_44100_hz_in_one_channel_at_128000_bit_per_second() {
    let settings = voice_encoder_settings();

    assert_eq!((settings.sample_rate_hz, settings.channels, settings.bit_rate), (44_100, 1, 128_000));
}
