use super::*;
use crate::api::MediaError;
use crate::memory::InputLimits;

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

fn size(width: u32, height: u32) -> crate::ffi::PixelSize {
    crate::ffi::PixelSize { width, height }
}

fn edit() -> PhotoEdit {
    PhotoEdit { max_size: None, enlarge: false, crop: None, output_size: None, mirror: false, rotation_degrees: 0 }
}

fn crop(x: f64, y: f64, width: f64, height: f64) -> Option<CropFractions> {
    Some(CropFractions { x, y, width, height })
}

#[test]
fn inv_send_21_a_photo_without_an_edit_keeps_the_upright_size_and_the_cost_of_the_plain_prepare() {
    let plain = photo_plan(400, 200, 1, edit()).unwrap();
    assert_eq!((plain.output, plain.crop, plain.mirror, plain.cost), (size(400, 200), PixelRect { x: 0, y: 0, width: 400, height: 200 }, false, 400 * 200 * 4));
    let turned = photo_plan(400, 200, 6, edit()).unwrap();
    assert_eq!((turned.upright, turned.output, turned.turn.rotation_degrees, turned.cost), (size(200, 400), size(200, 400), 90, 400 * 200 * 4 * 2));
}

#[test]
fn inv_send_22_the_max_size_fits_the_result_into_the_rectangle_and_never_enlarges() {
    for (source, limit, expected) in [((4000, 3000), (1080, 1920), (1080, 810)), ((3000, 4000), (1080, 1920), (1080, 1440)), ((100, 50), (1080, 1920), (100, 50)), ((1000, 100), (500, 500), (500, 50))] {
        let plan = photo_plan(source.0, source.1, 1, PhotoEdit { max_size: Some(size(limit.0, limit.1)), ..edit() }).unwrap();
        assert_eq!((plan.output.width, plan.output.height), expected, "{source:?} into {limit:?}");
    }
    assert_eq!(photo_plan(400, 200, 6, PhotoEdit { max_size: Some(size(100, 100)), ..edit() }).unwrap().output, size(50, 100));
    assert_eq!(photo_plan(400, 200, 1, PhotoEdit { max_size: Some(size(0, 100)), ..edit() }), None);
}

#[test]
fn inv_send_33_enlarge_fits_the_result_into_the_rectangle_with_the_proportions_and_may_grow_it() {
    for (source, limit, enlarge, expected) in [
        ((540, 960), (1080, 1920), true, (1080, 1920)),
        ((540, 960), (1080, 1920), false, (540, 960)),
        ((4000, 3000), (1080, 1920), true, (1080, 810)),
        ((4000, 3000), (1080, 1920), false, (1080, 810)),
        ((500, 500), (1080, 1920), true, (1080, 1080)),
        ((1000, 100), (500, 500), true, (500, 50)),
        ((100, 50), (16_384, 16_384), true, (16_384, 8_192)),
    ] {
        let plan = photo_plan(source.0, source.1, 1, PhotoEdit { max_size: Some(size(limit.0, limit.1)), enlarge, ..edit() }).unwrap();
        assert_eq!((plan.output.width, plan.output.height), expected, "{source:?} into {limit:?}, enlarge {enlarge}");
    }
    let grown = photo_plan(540, 960, 1, PhotoEdit { max_size: Some(size(1080, 1920)), enlarge: true, ..edit() }).unwrap();
    assert_eq!((grown.needs_draw, grown.cost), (true, 540 * 960 * 4 + 1080 * 1920 * 4));
    assert_eq!(photo_plan(100, 50, 1, PhotoEdit { max_size: Some(size(16_385, 16_385)), enlarge: true, ..edit() }), None);
}

#[test]
fn inv_send_23_the_crop_is_taken_from_the_upright_photo_and_held_inside_its_bounds() {
    let rect = |x, y, width, height| PixelRect { x, y, width, height };
    for (name, tag, fractions, expected) in [
        ("inside", 1, crop(0.25, 0.5, 0.5, 0.5), rect(250, 250, 500, 250)),
        ("past the edge", 1, crop(0.8, 0.8, 0.5, 0.5), rect(800, 400, 200, 100)),
        ("before the start", 1, crop(-0.5, 0.0, 1.0, 1.0), rect(0, 0, 500, 500)),
        ("after the turn", 6, crop(0.0, 0.0, 1.0, 0.5), rect(0, 0, 500, 500)),
    ] {
        let plan = photo_plan(1000, 500, tag, PhotoEdit { crop: fractions, ..edit() }).unwrap();
        assert_eq!(plan.crop, expected, "{name}");
        assert_eq!((plan.output.width, plan.output.height), (expected.width, expected.height), "{name}");
    }
    for (name, fractions) in [("outside", crop(1.2, 0.0, 0.5, 1.0)), ("empty", crop(0.2, 0.2, 0.0, 0.5)), ("not a number", crop(f64::NAN, 0.0, 1.0, 1.0))] {
        assert_eq!(photo_plan(1000, 500, 1, PhotoEdit { crop: fractions, ..edit() }), None, "{name}");
    }
}

#[test]
fn inv_send_24_the_output_size_is_exact_and_the_max_size_fits_the_crop() {
    let square = crop(0.0, 0.0, 0.5, 1.0);
    let exact = photo_plan(2000, 1000, 1, PhotoEdit { crop: square, output_size: Some(size(1080, 1080)), max_size: Some(size(10, 10)), enlarge: false, mirror: false, rotation_degrees: 0 }).unwrap();
    assert_eq!((exact.crop.width, exact.crop.height, exact.output), (1000, 1000, size(1080, 1080)));
    let fitted = photo_plan(2000, 1000, 1, PhotoEdit { crop: square, max_size: Some(size(100, 100)), ..edit() }).unwrap();
    assert_eq!(fitted.output, size(100, 100));
    assert_eq!(photo_plan(2000, 1000, 1, PhotoEdit { output_size: Some(size(0, 10)), ..edit() }), None);
}

#[test]
fn inv_send_25_the_mirror_is_carried_and_every_last_draw_adds_the_output_raster_to_the_cost() {
    let raster = 400 * 200 * 4;
    let mirrored = photo_plan(400, 200, 1, PhotoEdit { mirror: true, ..edit() }).unwrap();
    assert_eq!((mirrored.mirror, mirrored.cost), (true, raster * 2));
    let scaled = photo_plan(400, 200, 1, PhotoEdit { max_size: Some(size(200, 100)), ..edit() }).unwrap();
    assert_eq!(scaled.cost, 200 * 100 * 4 * 2);
    let turned_and_cut = photo_plan(400, 200, 6, PhotoEdit { crop: crop(0.0, 0.0, 1.0, 0.5), ..edit() }).unwrap();
    assert_eq!(turned_and_cut.cost, raster * 2 + 200 * 200 * 4);
    assert_eq!((turned_and_cut.sample, turned_and_cut.decode), (1, size(400, 200)));
}

#[test]
fn inv_send_26_a_photo_above_the_pixel_limit_or_an_output_above_it_has_no_plan() {
    assert_eq!(photo_plan(16_385, 10, 1, edit()), None);
    assert_eq!(photo_plan(0, 10, 1, edit()), None);
    assert_eq!(photo_plan(100, 100, 1, PhotoEdit { output_size: Some(size(16_385, 10)), ..edit() }), None);
}

#[test]
fn inv_send_27_the_plan_says_when_a_last_draw_is_needed() {
    assert!(!photo_plan(400, 200, 6, edit()).unwrap().needs_draw);
    for changed in [PhotoEdit { mirror: true, ..edit() }, PhotoEdit { max_size: Some(size(10, 10)), ..edit() }, PhotoEdit { crop: crop(0.0, 0.0, 0.5, 1.0), ..edit() }, PhotoEdit { output_size: Some(size(400, 201)), ..edit() }] {
        assert!(photo_plan(400, 200, 1, changed).unwrap().needs_draw, "{changed:?}");
    }
    assert!(!photo_plan(400, 200, 1, PhotoEdit { output_size: Some(size(400, 200)), ..edit() }).unwrap().needs_draw);
}

#[test]
fn inv_send_28_an_encode_plan_counts_one_raster_plus_one_per_redraw_and_flattens_only_a_jpeg_with_alpha() {
    let raster = 40 * 20 * 4;
    for (tag, alpha, format, flatten, cost) in [
        (1, false, ImageFormat::Png, false, raster),
        (1, true, ImageFormat::Png, false, raster),
        (1, true, ImageFormat::Jpeg, true, raster * 2),
        (1, false, ImageFormat::Jpeg, false, raster),
        (6, true, ImageFormat::Jpeg, true, raster * 3),
        (3, false, ImageFormat::Png, false, raster * 2),
    ] {
        let plan = image_encode_plan(40, 20, tag, alpha, format).unwrap();
        assert_eq!((plan.flatten, plan.cost), (flatten, cost), "tag {tag} alpha {alpha} {format:?}");
    }
    assert_eq!(image_encode_plan(16_385, 1, 1, false, ImageFormat::Png), None);
    assert_eq!(image_encode_plan(0, 1, 1, false, ImageFormat::Png), None);
}

#[test]
fn inv_send_42_an_encode_outside_a_layer_is_admitted_by_its_plan_cost_against_the_limit_and_refused_above_it() {
    let default = InputLimits::default().encode_bytes;
    let plan = image_encode_plan(1000, 1000, 6, true, ImageFormat::Jpeg).unwrap();
    assert_eq!(plan.cost, 12_000_000);
    assert_eq!(image_encode_outside_admit(plan.cost, None), Ok(()));
    assert_eq!(image_encode_outside_admit(plan.cost, Some(plan.cost)), Ok(()));
    let refused = image_encode_outside_admit(plan.cost, Some(plan.cost - 1)).unwrap_err();
    assert!(matches!(refused, MediaError::Overloaded { .. }), "{refused:?}");
    let large = image_encode_plan(2000, 2000, 6, true, ImageFormat::Jpeg).unwrap();
    assert!(large.cost > default && matches!(image_encode_outside_admit(large.cost, None), Err(MediaError::Overloaded { .. })));
}

#[test]
fn inv_send_34_a_crop_larger_than_the_output_is_decoded_down_to_it_and_the_ledger_pays_for_the_decoded_raster() {
    let square = crop(0.0, 0.0, 0.75, 1.0);
    let upper_half = crop(0.0, 0.0, 1.0, 0.5);
    let source = (8160, 6120);
    let fifty = source.0 * source.1;
    // (name, tag, edit, sample, decode, thumbnail side)
    let table = [
        ("visible square, output 1080", 1, PhotoEdit { crop: square, output_size: Some(size(1080, 1080)), ..edit() }, 4, size(2040, 1530), 1440),
        ("max size 1080x1920", 1, PhotoEdit { max_size: Some(size(1080, 1920)), ..edit() }, 4, size(2040, 1530), 1080),
        ("output size without crop", 1, PhotoEdit { output_size: Some(size(1080, 1080)), ..edit() }, 4, size(2040, 1530), 1440),
        ("tag 6: the crop is upright, the decode stays in file orientation", 6, PhotoEdit { crop: upper_half, output_size: Some(size(1080, 1080)), ..edit() }, 2, size(4080, 3060), 2160),
        ("tag 8", 8, PhotoEdit { crop: upper_half, output_size: Some(size(1080, 1080)), ..edit() }, 2, size(4080, 3060), 2160),
        ("mirror does not change the decode", 1, PhotoEdit { crop: square, output_size: Some(size(1080, 1080)), mirror: true, ..edit() }, 4, size(2040, 1530), 1440),
        ("output equal to the crop keeps the file size", 1, PhotoEdit { crop: square, output_size: Some(size(6120, 6120)), ..edit() }, 1, size(8160, 6120), 8160),
        ("output larger than the crop keeps the file size", 1, PhotoEdit { crop: square, output_size: Some(size(9000, 9000)), ..edit() }, 1, size(8160, 6120), 8160),
        ("enlarge keeps the file size", 1, PhotoEdit { max_size: Some(size(16_000, 16_000)), enlarge: true, ..edit() }, 1, size(8160, 6120), 8160),
        ("one output side above the half of the crop stops the sampling", 1, PhotoEdit { output_size: Some(size(4081, 100)), ..edit() }, 1, size(8160, 6120), 4081),
        ("both output sides at the half of the crop", 1, PhotoEdit { output_size: Some(size(4080, 3060)), ..edit() }, 2, size(4080, 3060), 4080),
        ("stretched output is held by the larger ratio", 1, PhotoEdit { output_size: Some(size(100, 3000)), ..edit() }, 2, size(4080, 3060), 4000),
        ("output of one pixel", 1, PhotoEdit { output_size: Some(size(1, 1)), ..edit() }, 4096, size(2, 2), 2),
    ];
    for (name, tag, changed, sample, decode, side) in table {
        let plan = photo_plan(source.0, source.1, tag, changed).unwrap();
        assert_eq!((plan.sample, plan.decode, plan.thumbnail_side), (sample, decode, side), "{name}");
        let turned = if tag == 1 { 1 } else { 2 };
        let draw = if plan.needs_draw { u64::from(plan.output.width) * u64::from(plan.output.height) * 4 } else { 0 };
        assert_eq!(plan.cost, u64::from(decode.width) * u64::from(decode.height) * 4 * turned + draw, "{name}");
    }
    let crop_plan = photo_plan(source.0, source.1, 1, PhotoEdit { crop: square, output_size: Some(size(1080, 1080)), ..edit() }).unwrap();
    assert!(u64::from(crop_plan.decode.width) * u64::from(crop_plan.decode.height) < u64::from(fifty) / 3);
}

#[test]
fn inv_send_34_the_pixel_limit_of_the_source_stays_when_the_decode_is_smaller() {
    let small = PhotoEdit { output_size: Some(size(128, 5)), ..edit() };
    assert_eq!(photo_plan(16_384, 10, 1, small).unwrap().decode, size(8192, 5));
    assert_eq!(photo_plan(16_385, 10, 1, small), None);
}

#[test]
fn inv_send_37_a_rotation_joins_the_exif_turn_in_one_turn_and_the_crop_is_taken_from_the_turned_photo() {
    let plain = photo_plan(400, 200, 1, PhotoEdit { rotation_degrees: 90, ..edit() }).unwrap();
    assert_eq!((plain.turn.rotation_degrees, plain.turn.flip_horizontal, plain.upright, plain.needs_draw), (90, false, size(200, 400), false));
    for (tag, rotation, expected) in [(6, 90, (180, false)), (6, 270, (0, false)), (3, 180, (0, false)), (5, 90, (0, true)), (5, 270, (180, true)), (2, 90, (270, true))] {
        let plan = photo_plan(400, 200, tag, PhotoEdit { rotation_degrees: rotation, ..edit() }).unwrap();
        assert_eq!((plan.turn.rotation_degrees, plan.turn.flip_horizontal), expected, "tag {tag} rotation {rotation}");
    }
    let cut = photo_plan(1000, 500, 1, PhotoEdit { rotation_degrees: 270, crop: crop(0.0, 0.0, 1.0, 0.5), ..edit() }).unwrap();
    assert_eq!((cut.upright, cut.crop), (size(500, 1000), PixelRect { x: 0, y: 0, width: 500, height: 500 }));
    assert_eq!(photo_plan(400, 200, 1, PhotoEdit { rotation_degrees: 45, ..edit() }), None);
    assert_eq!(photo_plan(400, 200, 1, PhotoEdit { rotation_degrees: 360, ..edit() }).unwrap().turn.rotation_degrees, 0);
    assert_eq!(plain.cost, 400 * 200 * 4 * 2);
}

#[test]
fn inv_send_38_a_video_with_location_or_personal_tags_must_be_copied_without_them_and_a_clean_one_stays() {
    let strip = |keys: &[&str]| video_metadata_to_strip(keys.iter().map(|key| key.to_string()).collect());
    for keys in [
        &["mdta/com.apple.quicktime.location.ISO6709"][..],
        &["com.apple.quicktime.location.accuracy.horizontal"],
        &["udta/©xyz"],
        &["©XYZ"],
        &["udta/loci"],
        &["location"],
        &["mdta/com.apple.quicktime.creationdate"],
        &["creation_time"],
        &["mdta/com.apple.quicktime.make", "mdta/com.apple.quicktime.model", "mdta/com.apple.quicktime.software"],
        &["udta/©nam", "itsk/©ART", "title", "comment"],
        &["major_brand", "mdta/com.apple.quicktime.location.ISO6709"],
    ] {
        assert!(!strip(keys).is_empty(), "{keys:?}");
    }
    assert_eq!(strip(&["major_brand", "mdta/com.apple.quicktime.location.ISO6709", "handler_name"]), ["mdta/com.apple.quicktime.location.ISO6709"]);
    for keys in [&[][..], &["major_brand", "minor_version", "compatible_brands"], &["udta/handler_name", " Encoder ", ""], &["itsk/©too"], &["TSSE"]] {
        assert!(strip(keys).is_empty(), "{keys:?}");
    }
}
