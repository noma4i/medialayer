use super::*;

const KNOWN: &str = "LEHV6nWB2yk8pyo0adR*.7kCMdnj";

#[test]
fn inv_placeholder_01_a_blurhash_decodes_to_rgba_bytes_and_a_bad_one_is_an_error_not_a_panic() {
    let pixels = decode_blurhash(KNOWN.to_string(), 32, 32, 1.0).unwrap();

    assert_eq!(pixels.len(), 32 * 32 * 4);
    assert!(matches!(decode_blurhash("short".to_string(), 32, 32, 1.0), Err(MediaError::BlurHash { .. })));
    assert!(matches!(decode_blurhash(KNOWN.to_string(), 0, 32, 1.0), Err(MediaError::BlurHash { .. })));
}

#[test]
fn inv_image_01_the_decode_size_rounds_placement_to_8_and_never_upscales_source() {
    let size = |width, height| PixelSize { width, height };
    let cases = [
        ((50.0, 50.0, 3.0, None), Some(size(152, 152))),
        ((96.0, 96.0, 3.0, None), Some(size(288, 288))),
        ((100.4, 20.0, 2.0, None), Some(size(208, 40))),
        ((50.0, 50.0, 3.0, Some(size(16, 16))), Some(size(16, 16))),
        ((0.0, 0.0, 3.0, Some(size(16, 16))), Some(size(16, 16))),
        ((0.0, 50.0, 3.0, None), None),
        ((50.0, -1.0, 3.0, None), None),
        ((50.0, 50.0, 0.0, None), None),
        ((f64::NAN, 50.0, 3.0, None), None),
        ((50.0, f64::INFINITY, 3.0, None), None),
        ((1.0e9, 50.0, 3.0, None), Some(size(16_384, 152))),
    ];
    for ((width, height, scale, declared), expected) in cases {
        assert_eq!(image_pixel_size(width, height, scale, declared, true), expected, "{width}x{height} at {scale} declared {declared:?}");
    }
}

#[test]
fn inv_image_11_placement_preserves_source_aspect_without_upscaling() {
    let source = Some(PixelSize { width: 1080, height: 1920 });
    for (crop, expected) in [(true, PixelSize { width: 352, height: 626 }), (false, PixelSize { width: 198, height: 352 })] {
        assert_eq!(image_pixel_size(352.0, 352.0, 1.0, source, crop), Some(expected));
    }
}

#[test]
fn inv_image_50_a_cancellation_settles_its_request_without_a_key() {
    let counters = ImageCounters::new();
    counters.record(ImageWork::Request { placement_pixels: 0 });
    counters.record(ImageWork::Cancel);
    let snapshot = counters.snapshot();

    assert_eq!((snapshot.active_requests, snapshot.cancellations), (0, 1));
}

#[test]
fn inv_image_37_cancellations_are_one_counter_not_a_map_of_every_url() {
    let counters = ImageCounters::new();
    for _ in 0..1000 {
        counters.record(ImageWork::Request { placement_pixels: 0 });
        counters.record(ImageWork::Cancel);
    }
    let snapshot = counters.snapshot();

    assert_eq!(snapshot.cancellations, 1000);
    assert_eq!(snapshot.active_requests, 0);
}
