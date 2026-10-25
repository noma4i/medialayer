use super::source::{bundled_source_error, is_local_url, local_file_url, normalize_item, remote_url, MediaItem};
use super::types::Kind;

#[test]
fn inv_source_01_blank_text_is_absent_and_the_url_loses_its_edge_whitespace() {
    let padded = MediaItem { cover: Some("  \n".to_string()), blur_hash: Some(" LEHV6nWB \t".to_string()), ..MediaItem::bare("  https://o.test/a.mp4\n", Kind::Video) };

    let normalized = normalize_item(padded);

    assert_eq!(normalized.url, "https://o.test/a.mp4");
    assert_eq!(normalized.cover, None);
    assert_eq!(normalized.blur_hash.as_deref(), Some("LEHV6nWB"));
    assert_eq!(normalize_item(MediaItem::bare(" \t ", Kind::Image)).url, "");
    assert!(!normalize_item(MediaItem::bare(" \t ", Kind::Image)).has_url());
}

#[test]
fn inv_source_02_a_path_or_file_url_becomes_one_canonical_file_url() {
    let cases = [
        ("an absolute path", "/tmp/a b.mp4", "file:///tmp/a%20b.mp4"),
        ("a padded path", "  /tmp/a.mp4 ", "file:///tmp/a.mp4"),
        ("a file url", "file:///tmp/a%20b.mp4", "file:///tmp/a%20b.mp4"),
        ("a relative path", "clips/a.mp4", ""),
        ("an empty text", "", ""),
        ("a web url", "https://o.test/a.mp4", ""),
    ];
    for (name, text, expected) in cases {
        assert_eq!(local_file_url(text.to_string()), expected, "{name}");
    }
}

#[test]
fn inv_source_03_only_a_valid_http_or_https_url_is_remote() {
    let cases = [
        ("https", " https://o.test/v.mp4 ", "https://o.test/v.mp4"),
        ("http", "http://o.test/v.mp4", "http://o.test/v.mp4"),
        ("another scheme", "ftp://o.test/v.mp4", ""),
        ("a file url", "file:///tmp/v.mp4", ""),
        ("no host", "https://", ""),
        ("plain text", "not a url", ""),
        ("blank", "   ", ""),
    ];
    for (name, text, expected) in cases {
        assert_eq!(remote_url(text.to_string()), expected, "{name}");
    }
}

#[test]
fn inv_source_04_a_bundled_image_is_refused_with_one_text() {
    assert_eq!(bundled_source_error(Kind::Image).as_deref(), Some("a bundled source is a video or an audio, not an image"));
    assert_eq!(bundled_source_error(Kind::Video), None);
    assert_eq!(bundled_source_error(Kind::Audio), None);
}

#[test]
fn inv_source_05_file_android_resource_and_asset_urls_are_local() {
    for url in ["file:///tmp/a.mp4", "android.resource://app.test/2131230720", "asset:///intro.mp3"] {
        assert!(is_local_url(url), "{url}");
        assert!(MediaItem::bare(url, Kind::Video).is_local(), "{url}");
    }
    for url in ["https://o.test/a.mp4", "http://o.test/a.mp4", "", "content://media/1"] {
        assert!(!is_local_url(url), "{url}");
    }
}
