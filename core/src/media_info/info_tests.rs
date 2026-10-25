use super::*;

fn size(width: u32, height: u32) -> Option<ImageSize> {
    Some(ImageSize { width, height })
}

#[test]
fn inv_info_01_a_quarter_turn_swaps_the_sides_and_a_half_turn_keeps_them() {
    for turn in [90, 270, -90, -270, 450] {
        assert_eq!(display_frame_size(1920, 1080, turn), size(1080, 1920), "{turn} degrees");
    }
    for turn in [0, 180, -180, 360] {
        assert_eq!(display_frame_size(1920, 1080, turn), size(1920, 1080), "{turn} degrees");
    }
}

#[test]
fn inv_info_01_a_frame_with_an_empty_side_has_no_size() {
    assert_eq!(display_frame_size(0, 1080, 90), None);
    assert_eq!(display_frame_size(1920, 0, 0), None);
}

#[test]
fn inv_info_02_only_the_local_schemes_name_a_file_with_metadata() {
    for url in ["file:///tmp/a.mp4", "android.resource://app/12", "asset:///clips/a.mp4", "  file:///tmp/a.mp4 "] {
        assert!(is_local_media_url(url.to_string()), "{url}");
    }
    for url in ["https://o.test/a.mp4", "http://o.test/a.mp4", "", "/tmp/a.mp4"] {
        assert!(!is_local_media_url(url.to_string()), "{url}");
    }
}
