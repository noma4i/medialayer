use super::descriptor::{decode_descriptor, encode_descriptor};
use crate::api::MediaError;
use super::*;
use crate::ffi::PixelSize;

fn source() -> VideoSource {
    VideoSource { width: 1280, height: 720, rotation_degrees: 0, frame_rate: 30.0, bitrate: Some(1_500_000), mp4_h264_aac: true }
}

fn edit() -> PhotoEdit {
    PhotoEdit { max_size: None, enlarge: false, crop: None, output_size: None, mirror: false, rotation_degrees: 0 }
}

fn video(mirror: bool, on_cannot_decode: CannotDecode) -> Vec<u8> {
    media_prepare_descriptor(MediaPrepareIntent::Video { limits: None, mirror, on_cannot_decode }).unwrap()
}

fn image(edit: PhotoEdit) -> Vec<u8> {
    media_prepare_descriptor(MediaPrepareIntent::Image { edit, quality: None, format: ImageFormat::Jpeg }).unwrap()
}

fn probe(source: VideoSource, keys: &[&str], size_bytes: u64) -> PrepareProbe {
    PrepareProbe::Video { source, metadata_keys: keys.iter().map(|key| key.to_string()).collect(), size_bytes, duration_ms: None }
}

fn photo(width: u32, height: u32) -> PrepareProbe {
    PrepareProbe::Image { width, height, exif_orientation: 1, format: ImageFormat::Jpeg }
}

fn file(width: u32, height: u32, mime: &str) -> MediaProbeOutcome {
    MediaProbeOutcome::File { width: Some(width), height: Some(height), mime: Some(mime.to_string()), duration_ms: None }
}

fn unsupported() -> MediaProbeOutcome {
    MediaProbeOutcome::File { width: None, height: None, mime: None, duration_ms: None }
}

#[test]
fn inv_send_51_a_file_at_or_below_the_recode_threshold_goes_as_it_is_and_a_larger_one_follows_the_plan() {
    let mut misfit = source();
    misfit.mp4_h264_aac = false;
    let with = |recode_above_bytes| VideoLimits { recode_above_bytes, ..default_video_limits() };
    for (threshold, size, reencode) in [(Some(1000), 1000, false), (Some(1000), 1001, true), (Some(0), 10, true), (None, 10, true)] {
        let plan = plan_video_sized(misfit, size, with(threshold)).unwrap();
        assert_eq!(plan.reencode, reencode, "{threshold:?} {size}");
    }
    assert!(!plan_video_sized(source(), 5, with(None)).unwrap().reencode);
    assert_eq!(plan_video(misfit, with(Some(1000))).unwrap().reencode, true);
}

#[test]
fn inv_send_52_the_descriptor_round_trips_every_intent_and_starts_with_the_version_byte() {
    let limits = VideoLimits { recode_above_bytes: Some(7_000_000), ..default_video_limits() };
    let crop = Some(CropFractions { x: 0.1, y: 0.2, width: 0.5, height: 0.25 });
    let full = PhotoEdit { max_size: Some(PixelSize { width: 800, height: 600 }), enlarge: true, crop, output_size: Some(PixelSize { width: 64, height: 64 }), mirror: true, rotation_degrees: 270 };
    let intents = [
        MediaPrepareIntent::Video { limits: None, mirror: false, on_cannot_decode: CannotDecode::Fail },
        MediaPrepareIntent::Video { limits: Some(limits), mirror: true, on_cannot_decode: CannotDecode::UploadSource },
        MediaPrepareIntent::Image { edit: edit(), quality: None, format: ImageFormat::Jpeg },
        MediaPrepareIntent::Image { edit: full, quality: Some(0.75), format: ImageFormat::Png },
    ];
    for intent in intents {
        let bytes = media_prepare_descriptor(intent.clone()).unwrap();
        assert_eq!(bytes[0], 1);
        assert_eq!(decode_descriptor(&bytes), Ok(intent));
    }
}

#[test]
fn inv_send_53_an_unknown_version_a_short_or_a_longer_descriptor_is_unsupported_and_never_a_crash() {
    let good = video(false, CannotDecode::Fail);
    let mut other_version = good.clone();
    other_version[0] = 2;
    let mut longer = good.clone();
    longer.push(0);
    let mut unknown_kind = good.clone();
    unknown_kind[1] = 9;
    let mut huge_list = vec![1, 0, 1];
    huge_list.extend([0, 0, 0, 10, 0xff, 0xff, 0xff, 0xff]);
    for bytes in [other_version, longer, unknown_kind, huge_list, Vec::new(), vec![1], good[..good.len() - 1].to_vec()] {
        assert_eq!(decode_descriptor(&bytes), Err(MediaPrepareFailure::UnsupportedDescriptor), "{bytes:?}");
        assert!(matches!(media_prepare_decide(bytes.clone(), photo(10, 10)), PrepareDecision::Refuse { reason: MediaPrepareFailure::UnsupportedDescriptor, .. }));
        assert_eq!(media_prepare_facts(Some(bytes), photo(10, 10)), unsupported());
    }
}

#[test]
fn inv_send_54_a_video_that_fits_and_is_clean_passes_and_the_rest_runs_with_the_audio_settings_of_the_limits() {
    let plain = video(false, CannotDecode::Fail);
    assert_eq!(media_prepare_decide(plain.clone(), probe(source(), &["major_brand"], 1)), PrepareDecision::Pass);
    let limits = default_video_limits();
    for (descriptor, keys, name) in [(plain.clone(), &["mdta/com.apple.quicktime.location.ISO6709"][..], "metadata"), (video(true, CannotDecode::Fail), &[][..], "mirror")] {
        match media_prepare_decide(descriptor, probe(source(), keys, 1)) {
            PrepareDecision::Run { plan: PreparePlan::Video { plan, strip_metadata, audio_bitrate, audio_channels, audio_sample_rate, mirror } } => {
                assert_eq!((audio_bitrate, audio_channels, audio_sample_rate), (limits.audio_bitrate, limits.audio_channels, limits.audio_sample_rate), "{name}");
                assert_eq!((plan.reencode, strip_metadata.is_empty(), mirror), (name == "mirror", name == "mirror", name == "mirror"), "{name}");
            }
            other => panic!("{name}: {other:?}"),
        }
    }
    let mut turned = source();
    turned.rotation_degrees = 90;
    assert!(matches!(media_prepare_decide(plain, probe(turned, &[], 1)), PrepareDecision::Run { plan: PreparePlan::Video { plan, .. } } if plan.reencode));
}

#[test]
fn inv_send_55_a_source_the_device_cannot_decode_follows_the_policy_of_the_descriptor() {
    let failed = PrepareProbe::Failed { message: "no".to_string() };
    assert_eq!(media_prepare_decide(video(false, CannotDecode::UploadSource), failed.clone()), PrepareDecision::Pass);
    assert!(matches!(media_prepare_decide(video(false, CannotDecode::Fail), failed.clone()), PrepareDecision::Refuse { reason: MediaPrepareFailure::CannotDecode, .. }));
    assert!(matches!(media_prepare_decide(video(false, CannotDecode::Fail), photo(10, 10)), PrepareDecision::Refuse { reason: MediaPrepareFailure::CannotDecode, .. }));
    assert!(matches!(media_prepare_decide(image(edit()), failed), PrepareDecision::Refuse { reason: MediaPrepareFailure::CannotDecode, .. }));
    assert!(matches!(media_prepare_decide(image(edit()), photo(20_000, 10)), PrepareDecision::Refuse { reason: MediaPrepareFailure::CannotDecode, .. }));
    match media_prepare_decide(image(PhotoEdit { max_size: Some(PixelSize { width: 50, height: 50 }), ..edit() }), photo(200, 100)) {
        PrepareDecision::Run { plan: PreparePlan::Image { plan, quality, format } } => assert_eq!((plan.output, quality, format), (PixelSize { width: 50, height: 25 }, 0.9, ImageFormat::Jpeg)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn inv_send_56_the_planned_size_and_mime_apply_the_turn_the_mirror_and_the_crop_and_hold_for_a_pass() {
    let facts = |descriptor, probe| media_prepare_facts(Some(descriptor), probe);
    assert_eq!(facts(video(false, CannotDecode::Fail), probe(source(), &[], 1)), file(1280, 720, "video/mp4"));
    let mut turned = source();
    turned.rotation_degrees = 90;
    assert_eq!(facts(video(false, CannotDecode::Fail), probe(turned, &[], 1)), file(720, 1280, "video/mp4"));
    let crop = PhotoEdit { crop: Some(CropFractions { x: 0.0, y: 0.0, width: 0.5, height: 1.0 }), ..edit() };
    assert_eq!(facts(image(crop), photo(200, 100)), file(100, 100, "image/jpeg"));
    assert_eq!(facts(image(edit()), probe(source(), &[], 1)), unsupported());
    assert_eq!(facts(image(edit()), PrepareProbe::Failed { message: "no".to_string() }), MediaProbeOutcome::Failed { message: "no".to_string() });
}

#[test]
fn inv_send_57_the_facts_without_a_descriptor_are_the_upright_source_as_the_platform_reports_it() {
    let facts = |probe| media_prepare_facts(None, probe);
    let mut turned = source();
    turned.rotation_degrees = 270;
    assert_eq!(facts(probe(turned, &[], 1)), file(720, 1280, "video/mp4"));
    assert_eq!(facts(PrepareProbe::Image { width: 200, height: 100, exif_orientation: 6, format: ImageFormat::Png }), file(100, 200, "image/png"));
    assert_eq!(facts(PrepareProbe::Failed { message: "no".to_string() }), MediaProbeOutcome::Failed { message: "no".to_string() });
}

#[test]
fn inv_send_58_settings_the_planner_cannot_work_with_are_refused_before_bytes_exist() {
    let bad_limits = VideoLimits { corridors: Vec::new(), ..default_video_limits() };
    let image = |edit, quality| MediaPrepareIntent::Image { edit, quality, format: ImageFormat::Jpeg };
    let intents = [
        MediaPrepareIntent::Video { limits: Some(bad_limits), mirror: false, on_cannot_decode: CannotDecode::Fail },
        image(edit(), Some(1.5)),
        image(PhotoEdit { max_size: Some(PixelSize { width: 0, height: 5 }), ..edit() }, None),
        image(PhotoEdit { crop: Some(CropFractions { x: 0.0, y: 0.0, width: 0.0, height: 1.0 }), ..edit() }, None),
        image(PhotoEdit { rotation_degrees: 45, ..edit() }, None),
    ];
    for intent in intents {
        assert!(matches!(media_prepare_descriptor(intent.clone()), Err(MediaError::InvalidSpec { .. })), "{intent:?}");
        // Hand-made bytes of the same intent still reach decide and are refused there by the same check.
        assert!(matches!(media_prepare_decide(encode_descriptor(intent), photo(10, 10)), PrepareDecision::Refuse { reason: MediaPrepareFailure::UnsupportedDescriptor, .. }));
    }
}

#[test]
fn inv_send_59_a_refusal_maps_to_cannot_decode_or_to_a_failure_with_the_message() {
    let message = || "m".to_string();
    assert_eq!(MediaPrepareOutcome::refused(MediaPrepareFailure::CannotDecode, message()), MediaPrepareOutcome::CannotDecode { message: message() });
    for reason in [MediaPrepareFailure::SourceUnreadable, MediaPrepareFailure::WriteFailed, MediaPrepareFailure::UnsupportedDescriptor] {
        assert_eq!(MediaPrepareOutcome::refused(reason, message()), MediaPrepareOutcome::Failed { message: message() });
    }
}

#[test]
fn inv_send_60_a_pass_is_answered_by_unchanged_and_never_by_a_written_destination() {
    let unchanged = MediaPrepareOutcome::Unchanged { name: None, mime: None };
    assert_ne!(unchanged, MediaPrepareOutcome::Done { name: None, mime: None });
    assert_eq!(media_prepare_decide(video(false, CannotDecode::UploadSource), PrepareProbe::Failed { message: "no".to_string() }), PrepareDecision::Pass);
}

#[test]
fn inv_send_64_the_descriptor_kind_is_known_without_the_source_and_garbage_has_none() {
    assert_eq!(media_prepare_descriptor_kind(video(false, CannotDecode::Fail)), Some(PrepareKind::Video));
    assert_eq!(media_prepare_descriptor_kind(image(edit())), Some(PrepareKind::Image));
    assert_eq!(media_prepare_descriptor_kind(vec![9, 9, 9]), None);
    assert_eq!(media_prepare_descriptor_kind(Vec::new()), None);
}

#[test]
fn inv_send_57_the_probe_facts_carry_the_video_duration_and_an_image_has_none() {
    let timed = PrepareProbe::Video { source: source(), metadata_keys: Vec::new(), size_bytes: 1, duration_ms: Some(1234) };
    let video_file = MediaProbeOutcome::File { width: Some(1280), height: Some(720), mime: Some("video/mp4".to_string()), duration_ms: Some(1234) };
    assert_eq!(media_prepare_facts(None, timed.clone()), video_file);
    assert_eq!(media_prepare_facts(Some(video(false, CannotDecode::Fail)), timed), video_file);
    assert_eq!(media_prepare_facts(None, photo(200, 100)), file(200, 100, "image/jpeg"));
}
