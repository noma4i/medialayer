use super::route::*;

#[test]
fn inv_proxy_16_proxy_url_carries_the_encoded_original_with_the_extension() {
    assert_eq!(
        proxy_url(5000, "https://a.com/v/p.m3u8?t=1"),
        "http://127.0.0.1:5000/request.m3u8?url=https%3A%2F%2Fa.com%2Fv%2Fp.m3u8%3Ft%3D1"
    );
    assert_eq!(proxy_url(1, "https://a.com/v/clip.mp4"), "http://127.0.0.1:1/request.mp4?url=https%3A%2F%2Fa.com%2Fv%2Fclip.mp4");
    assert!(proxy_url(1, "https://a.com/stream").contains("/request.bin?"));
}

#[test]
fn inv_proxy_16_target_url_reads_the_query_form() {
    assert_eq!(
        target_url("/request.m3u8", Some("url=https%3A%2F%2Fa.com%2Fp.m3u8%3Ft%3D1")),
        Some("https://a.com/p.m3u8?t=1".to_string())
    );
}

#[test]
fn inv_proxy_16_target_url_reads_the_rewritten_path_form() {
    assert_eq!(target_url("/https://cdn/a.ts", None), Some("https://cdn/a.ts".to_string()));
    assert_eq!(target_url("/./https://cdn/a.ts", Some("x=1")), Some("https://cdn/a.ts?x=1".to_string()));
    assert_eq!(target_url("/http://cdn/a.ts", None), Some("http://cdn/a.ts".to_string()));
    assert_eq!(
        target_url("/https://o.test/hls/https://cdn.test/seg0.ts", Some("t=1")),
        Some("https://cdn.test/seg0.ts?t=1".to_string())
    );
}

#[test]
fn inv_proxy_16_target_url_rejects_unknown_routes() {
    assert_eq!(target_url("/random", None), None);
    assert_eq!(target_url("/request.mp4", None), None);
    assert_eq!(target_url("/request.mp4", Some("other=1")), None);
    assert_eq!(target_url("/", None), None);
}

#[test]
fn inv_proxy_16_parses_single_byte_ranges() {
    assert_eq!(parse_range("bytes=10-19"), Some(ByteRange::From { start: 10, end: Some(19) }));
    assert_eq!(parse_range("bytes=10-"), Some(ByteRange::From { start: 10, end: None }));
    assert_eq!(parse_range("bytes=-5"), Some(ByteRange::Suffix(5)));
    assert_eq!(parse_range("bytes=0-1,5-6"), None);
    assert_eq!(parse_range("items=0-1"), None);
    assert_eq!(parse_range("bytes=a-b"), None);
    assert_eq!(parse_range("bytes=5-2"), None);
}

#[test]
fn inv_proxy_16_resolves_ranges_against_the_total() {
    assert_eq!(resolve_range(None, 100), RangeOutcome::Full);
    assert_eq!(resolve_range(Some(ByteRange::From { start: 10, end: Some(19) }), 100), RangeOutcome::Partial { start: 10, end: 19 });
    assert_eq!(resolve_range(Some(ByteRange::From { start: 90, end: None }), 100), RangeOutcome::Partial { start: 90, end: 99 });
    assert_eq!(resolve_range(Some(ByteRange::From { start: 90, end: Some(500) }), 100), RangeOutcome::Partial { start: 90, end: 99 });
    assert_eq!(resolve_range(Some(ByteRange::Suffix(5)), 100), RangeOutcome::Partial { start: 95, end: 99 });
    assert_eq!(resolve_range(Some(ByteRange::Suffix(500)), 100), RangeOutcome::Partial { start: 0, end: 99 });
    assert_eq!(resolve_range(Some(ByteRange::From { start: 100, end: None }), 100), RangeOutcome::Unsatisfiable);
}

#[test]
fn inv_proxy_16_playlist_references_list_segment_and_tag_urls() {
    let text = "#EXTM3U\n#EXT-X-MAP:URI=\"https://a/init.mp4\"\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:2,\nhttps://a/s0.ts\r\n\n#EXT-X-KEY:URI=\"skd://k\"\n";
    assert_eq!(crate::hls::Playlist::parse(text, "").references, vec!["https://a/init.mp4".to_string(), "https://a/s0.ts".to_string()]);
}

#[test]
fn inv_proxy_16_content_type_prefers_the_origin_then_the_extension() {
    assert_eq!(content_type_for("https://a.com/x.mp4", Some("text/plain")), "text/plain");
    assert_eq!(content_type_for("https://a.com/x.ts?t=1", None), "video/mp2t");
    assert_eq!(content_type_for("https://a.com/x.dat", Some("image/png")), "image/png");
    assert_eq!(content_type_for("https://a.com/x", None), "application/octet-stream");
}

#[test]
fn inv_proxy_16_absolutize_resolves_relative_tag_uris_and_keeps_absolute_and_foreign_ones() {
    let text = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"1@0\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/key\"\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\"\n#EXT-X-MEDIA:URI=\"../a/a.m3u8\"\n";
    assert_eq!(
        crate::hls::Playlist::parse(text, "https://o.com/v/p.m3u8").body,
        "#EXTM3U\n#EXT-X-MAP:URI=\"./https://o.com/v/init.mp4\",BYTERANGE=\"1@0\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"./https://k/key\"\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\"\n#EXT-X-MEDIA:URI=\"./https://o.com/a/a.m3u8\"\n"
    );
}
