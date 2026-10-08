use super::*;

const MULTIVARIANT: &str =
    "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2400000\nhigh/index.m3u8\n";

fn leading(manifest: &str, min: f64) -> LeadingSegments {
    extract_leading_segments(manifest, min)
}

#[test]
fn detects_multivariant_manifest() {
    assert!(is_multivariant(MULTIVARIANT));
    assert!(!is_multivariant("#EXTM3U\n#EXTINF:2.0,\na.ts"));
}

#[test]
fn variants_keep_order() {
    assert_eq!(extract_variants(MULTIVARIANT), vec!["low/index.m3u8", "high/index.m3u8"]);
}

#[test]
fn variants_skip_comments_after_stream_inf() {
    let manifest = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\n#comment\n#EXT-X-STREAM-INF:BANDWIDTH=2\nhigh.m3u8\n";
    assert_eq!(extract_variants(manifest), vec!["high.m3u8"]);
}

#[test]
fn variants_handle_stream_inf_on_the_last_line() {
    assert!(extract_variants("#EXT-X-STREAM-INF:BANDWIDTH=1").is_empty());
}

#[test]
fn stops_once_duration_is_covered() {
    let manifest = "#EXTM3U\n#EXTINF:1.0,\na.ts\n#EXTINF:1.0,\nb.ts\n#EXTINF:1.0,\nc.ts\n";
    let selection = leading(manifest, 2.0);
    assert_eq!(selection.segments, vec!["a.ts", "b.ts"]);
    assert_eq!(selection.init_segment, None);
}

#[test]
fn takes_one_long_segment() {
    let manifest = "#EXTM3U\n#EXTINF:6.0,\na.ts\n#EXTINF:6.0,\nb.ts\n";
    assert_eq!(leading(manifest, 2.0).segments, vec!["a.ts"]);
}

#[test]
fn reads_init_segment_from_map_uri() {
    let manifest = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"0@0\"\n#EXTINF:4.0,\na.m4s\n";
    let selection = leading(manifest, 2.0);
    assert_eq!(selection.init_segment, Some("init.mp4".to_string()));
    assert_eq!(selection.segments, vec!["a.m4s"]);
}

#[test]
fn caps_at_two_segments_when_a_duration_is_missing() {
    let manifest = "#EXTM3U\n#EXTINF:1.0,\na.ts\nb.ts\n#EXTINF:1.0,\nc.ts\n";
    assert_eq!(leading(manifest, 60.0).segments, vec!["a.ts", "b.ts"]);
}

#[test]
fn treats_invalid_or_negative_duration_as_missing() {
    let manifest = "#EXTM3U\n#EXTINF:-1,\na.ts\n#EXTINF:abc,\nb.ts\n#EXTINF:1.0,\nc.ts\n";
    assert_eq!(leading(manifest, 60.0).segments, vec!["a.ts", "b.ts"]);
}

#[test]
fn handles_crlf_and_blank_lines() {
    let manifest = "#EXTM3U\r\n\r\n#EXTINF:1.0,\r\na.ts\r\n\r\n#EXTINF:1.0,\r\nb.ts\r\n#EXTINF:1.0,\r\nc.ts\r\n";
    assert_eq!(leading(manifest, 2.0).segments, vec!["a.ts", "b.ts"]);
}

#[test]
fn reads_map_uri_with_crlf_and_ignores_map_without_quotes() {
    let mapped = "#EXT-X-MAP:URI=\"init.mp4\"\r\n#EXTINF:4.0,\r\na.m4s\r\n";
    assert_eq!(leading(mapped, 2.0).init_segment, Some("init.mp4".to_string()));
    let unquoted = "#EXT-X-MAP:URI=init.mp4\n#EXTINF:4.0,\na.m4s\n";
    assert_eq!(leading(unquoted, 2.0).init_segment, None);
    let unclosed = "#EXT-X-MAP:URI=\"init.mp4\n#EXTINF:4.0,\na.m4s\n";
    assert_eq!(leading(unclosed, 2.0).init_segment, None);
}

#[test]
fn keeps_segment_queries_and_absolute_urls() {
    let manifest = "#EXTM3U\n#EXTINF:3.0,\na.ts?token=1&x=2\n#EXTINF:3.0,\nhttps://h/b.ts\n";
    assert_eq!(leading(manifest, 60.0).segments, vec!["a.ts?token=1&x=2", "https://h/b.ts"]);
}

#[test]
fn empty_manifest_has_no_segments() {
    assert_eq!(leading("", 2.0), LeadingSegments::default());
}

#[test]
fn extinf_without_a_comma_still_has_a_duration() {
    let manifest = "#EXTINF:2.5\na.ts\n#EXTINF:2.5\nb.ts\n";
    assert_eq!(leading(manifest, 4.0).segments, vec!["a.ts", "b.ts"]);
}

#[test]
fn resolves_relative_path_against_base() {
    assert_eq!(resolve_url("https://h/a/b/index.m3u8", "seg.ts"), "https://h/a/b/seg.ts");
    assert_eq!(resolve_url("https://h/a/b/index.m3u8?t=1", "seg.ts?x=2"), "https://h/a/b/seg.ts?x=2");
}

#[test]
fn resolves_parent_paths() {
    assert_eq!(resolve_url("https://h/a/b/index.m3u8", "../seg.ts"), "https://h/a/seg.ts");
    assert_eq!(resolve_url("https://h/a/b/index.m3u8", "../../../seg.ts"), "https://h/seg.ts");
}

#[test]
fn resolves_absolute_and_root_paths() {
    assert_eq!(resolve_url("https://h/a/index.m3u8", "https://o/x.ts"), "https://o/x.ts");
    assert_eq!(resolve_url("https://h/a/index.m3u8", "/x.ts"), "https://h/x.ts");
}

#[test]
fn returns_the_relative_text_when_the_base_is_invalid() {
    assert_eq!(resolve_url("not a url", "seg.ts"), "seg.ts");
}
