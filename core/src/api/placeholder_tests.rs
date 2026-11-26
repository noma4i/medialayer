use super::*;
use std::collections::HashMap;

fn source(kind: Kind, cover: Option<&str>, blur_hash: Option<&str>) -> MediaItem {
    MediaItem {
        kind,
        url: "https://cdn.test/a".to_string(),
        headers: HashMap::new(),
        cover: cover.map(str::to_string),
        blur_hash: blur_hash.map(str::to_string),
        looping: true,
        strategy: crate::prefetch::CacheStrategy::Window,
    }
}

#[test]
fn inv_placeholder_02_a_blur_hash_over_the_input_limit_is_not_handed_to_the_platform() {
    let oversized = "L".repeat(10_000);

    let placeholder = placeholder_for(source(Kind::Image, None, Some(&oversized)), false);

    assert!(matches!(placeholder, Placeholder::None));
}

#[test]
fn inv_placeholder_02_b_a_video_blur_hash_over_the_input_limit_falls_back_to_the_first_frame() {
    let oversized = "L".repeat(10_000);

    assert_eq!(placeholder_for(source(Kind::Video, None, Some(&oversized)), false), Placeholder::FirstFrame);
}

#[test]
fn inv_placeholder_02_the_placeholder_follows_the_kind_the_optional_fields_and_the_shown_frame() {
    let hash = || Placeholder::BlurHash { hash: "LEHV6n".to_string() };
    let cover = || Placeholder::Cover { url: "https://cdn.test/c.jpg".to_string() };
    let cases = [
        ("image with a blur hash shows it", Kind::Image, None, Some("LEHV6n"), false, hash()),
        ("image without a blur hash stays empty without an error", Kind::Image, None, None, false, Placeholder::None),
        ("a loaded image drops the blur hash", Kind::Image, None, Some("LEHV6n"), true, Placeholder::None),
        ("video with a cover shows the cover", Kind::Video, Some("https://cdn.test/c.jpg"), None, false, cover()),
        ("remote video without a cover shows its blur hash", Kind::Video, None, Some("LEHV6n"), false, hash()),
        ("remote video with a cover and a blur hash shows the cover", Kind::Video, Some("https://cdn.test/c.jpg"), Some("LEHV6n"), false, cover()),
        ("a shown video frame drops the blur hash", Kind::Video, None, Some("LEHV6n"), true, Placeholder::None),
        ("video without a cover shows the first frame", Kind::Video, None, None, false, Placeholder::FirstFrame),
        ("audio has no placeholder", Kind::Audio, None, Some("LEHV6n"), false, Placeholder::None),
    ];

    for (name, kind, cover_url, blur, ready, expected) in cases {
        assert_eq!(placeholder_for(source(kind, cover_url, blur), ready), expected, "{name}");
    }
}

#[test]
fn inv_placeholder_09_a_video_keeps_its_cover_until_the_first_frame_is_shown() {
    let cover = || Placeholder::Cover { url: "https://cdn.test/c.jpg".to_string() };
    let video = || source(Kind::Video, Some("https://cdn.test/c.jpg"), None);
    let cases = [
        ("loading: the first frame is not shown", false, cover()),
        ("ready: the layer is still transparent", false, cover()),
        ("failed: nothing was shown", false, cover()),
        ("rendered: the first frame is on screen", true, Placeholder::None),
        ("ended: the last frame is on screen", true, Placeholder::None),
    ];

    for (state, ready, expected) in cases {
        assert_eq!(placeholder_for(video(), ready), expected, "{state}");
    }
}

#[test]
fn inv_placeholder_09_a_local_video_shows_no_cover_even_before_its_first_frame() {
    let local = MediaItem { cover: Some("https://cdn.test/c.jpg".to_string()), url: "file:///bundle/intro.mp4".to_string(), ..source(Kind::Video, None, None) };

    assert_eq!(placeholder_for(local, false), Placeholder::None);
}

#[test]
fn inv_placeholder_11_a_local_video_without_a_cover_shows_its_first_frame_until_it_is_rendered() {
    let local = || MediaItem { url: "file:///picked/clip.mp4".to_string(), ..source(Kind::Video, None, None) };

    assert_eq!(placeholder_for(local(), false), Placeholder::FirstFrame);
    assert_eq!(placeholder_for(local(), true), Placeholder::None);
}
