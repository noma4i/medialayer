use super::*;

const MULTIVARIANT: &str =
    "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2400000\nhigh/index.m3u8\n";

#[test]
fn inv_hls_05_detects_multivariant_manifest() {
    assert!(is_multivariant(MULTIVARIANT));
    assert!(!is_multivariant("#EXTM3U\n#EXTINF:2.0,\na.ts"));
}

#[test]
fn inv_hls_05_variants_keep_order() {
    assert_eq!(extract_variants(MULTIVARIANT), vec!["low/index.m3u8", "high/index.m3u8"]);
}

#[test]
fn inv_hls_05_variants_skip_comments_after_stream_inf() {
    let manifest = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1\n#comment\n#EXT-X-STREAM-INF:BANDWIDTH=2\nhigh.m3u8\n";
    assert_eq!(extract_variants(manifest), vec!["high.m3u8"]);
}

#[test]
fn inv_hls_05_variants_handle_stream_inf_on_the_last_line() {
    assert!(extract_variants("#EXT-X-STREAM-INF:BANDWIDTH=1").is_empty());
}

fn segments(manifest: &str, max: usize) -> Vec<String> {
    extract_leading_chunks(manifest, max).into_iter().map(|chunk| chunk.segment).collect()
}

#[test]
fn inv_hls_02_takes_the_first_n_segments_and_never_more_than_the_playlist_has() {
    let manifest = "#EXTM3U\n#EXTINF:1.0,\na.ts\n#EXTINF:1.0,\nb.ts\nc.ts\n#EXTINF:abc,\nd.ts\n";
    let cases: [(usize, &[&str]); 4] = [(1, &["a.ts"]), (2, &["a.ts", "b.ts"]), (4, &["a.ts", "b.ts", "c.ts", "d.ts"]), (8, &["a.ts", "b.ts", "c.ts", "d.ts"])];
    for (max, expected) in cases {
        assert_eq!(segments(manifest, max), expected, "max {max}");
    }
    assert!(extract_leading_chunks("", 3).is_empty());
}

#[test]
fn inv_hls_01_each_chunk_carries_the_map_that_applies_to_it() {
    let playlist = "#EXTM3U\n#EXT-X-MAP:URI=\"init0.mp4\",BYTERANGE=\"0@0\"\r\n#EXTINF:2,\ns0.m4s\n#EXT-X-DISCONTINUITY\n#EXT-X-MAP:URI=\"init1.mp4\"\n#EXTINF:2,\ns1.m4s\n";
    let chunks = extract_leading_chunks(playlist, 8);
    assert_eq!(chunks, vec![Chunk { segment: "s0.m4s".into(), init: Some("init0.mp4".into()) }, Chunk { segment: "s1.m4s".into(), init: Some("init1.mp4".into()) }]);
    let unquoted = "#EXT-X-MAP:URI=init.mp4\n#EXTINF:4.0,\na.m4s\n";
    assert_eq!(extract_leading_chunks(unquoted, 8)[0].init, None);
}

#[test]
fn inv_hls_05_handles_crlf_blank_lines_queries_and_absolute_urls() {
    let manifest = "#EXTM3U\r\n\r\n#EXTINF:1.0,\r\na.ts?token=1&x=2\r\n\r\n#EXTINF:1.0,\r\nhttps://h/b.ts\r\n";
    assert_eq!(segments(manifest, 8), vec!["a.ts?token=1&x=2", "https://h/b.ts"]);
}

#[test]
fn inv_hls_05_resolve_url_joins_relative_parent_absolute_and_root_paths() {
    let cases = [
        ("https://h/a/b/index.m3u8", "seg.ts", "https://h/a/b/seg.ts"),
        ("https://h/a/b/index.m3u8?t=1", "seg.ts?x=2", "https://h/a/b/seg.ts?x=2"),
        ("https://h/a/b/index.m3u8", "../seg.ts", "https://h/a/seg.ts"),
        ("https://h/a/b/index.m3u8", "../../../seg.ts", "https://h/seg.ts"),
        ("https://h/a/index.m3u8", "https://o/x.ts", "https://o/x.ts"),
        ("https://h/a/index.m3u8", "/x.ts", "https://h/x.ts"),
        ("not a url", "seg.ts", "seg.ts"),
    ];
    for (base, relative, expected) in cases {
        assert_eq!(resolve_url(base, relative), expected, "{base} + {relative}");
    }
}
