use super::*;
use std::collections::HashMap;

fn source(kind: Kind, cover: Option<&str>, blur_hash: Option<&str>) -> MediaItem {
    MediaItem {
        kind,
        url: "https://cdn.test/a".to_string(),
        headers: HashMap::new(),
        cover: cover.map(str::to_string),
        blur_hash: blur_hash.map(str::to_string),
    }
}

#[test]
fn a_blur_hash_over_the_input_limit_is_not_handed_to_the_platform() {
    let oversized = "L".repeat(10_000);

    let placeholder = placeholder_for(source(Kind::Image, None, Some(&oversized)), CacheState::Missing, false);

    assert!(matches!(placeholder, Placeholder::None));
}

#[test]
fn the_placeholder_follows_the_kind_the_optional_fields_and_the_cache() {
    let hash = || Placeholder::BlurHash { hash: "LEHV6n".to_string() };
    let cover = || Placeholder::Cover { url: "https://cdn.test/c.jpg".to_string() };
    let cases = [
        ("image with a blur hash shows it", Kind::Image, None, Some("LEHV6n"), CacheState::Missing, false, hash()),
        ("image without a blur hash stays empty without an error", Kind::Image, None, None, CacheState::Missing, false, Placeholder::None),
        ("a loaded image drops the blur hash", Kind::Image, None, Some("LEHV6n"), CacheState::Cached, true, Placeholder::None),
        ("video with a cover shows the cover", Kind::Video, Some("https://cdn.test/c.jpg"), None, CacheState::Missing, false, cover()),
        ("video never uses a blur hash", Kind::Video, None, Some("LEHV6n"), CacheState::Missing, false, Placeholder::FirstFrame),
        ("video without a cover shows the first frame", Kind::Video, None, None, CacheState::Missing, false, Placeholder::FirstFrame),
        ("cached video skips the placeholder", Kind::Video, Some("https://cdn.test/c.jpg"), None, CacheState::Cached, false, Placeholder::None),
        ("a ready slot skips the placeholder", Kind::Video, None, None, CacheState::Missing, true, Placeholder::None),
        ("audio has no placeholder", Kind::Audio, None, Some("LEHV6n"), CacheState::Missing, false, Placeholder::None),
    ];

    for (name, kind, cover_url, blur, cache, ready, expected) in cases {
        assert_eq!(placeholder_for(source(kind, cover_url, blur), cache, ready), expected, "{name}");
    }
}
