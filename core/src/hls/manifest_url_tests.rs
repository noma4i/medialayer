use super::*;

#[test]
fn matches_manifest_extension_ignoring_case() {
    assert!(is_manifest_url("https://cdn.example.com/a/index.m3u8"));
    assert!(is_manifest_url("https://cdn.example.com/a/INDEX.M3U8"));
}

#[test]
fn ignores_query_and_fragment() {
    assert!(is_manifest_url("https://cdn.example.com/index.m3u8?token=1"));
    assert!(is_manifest_url("https://cdn.example.com/index.m3u8#t=3"));
    assert!(is_manifest_url("https://cdn.example.com/index.m3u8?a=b#frag"));
}

#[test]
fn rejects_other_extensions_and_manifest_text_outside_the_path() {
    assert!(!is_manifest_url("https://cdn.example.com/video.mp4"));
    assert!(!is_manifest_url("https://cdn.example.com/video.mp4?src=index.m3u8"));
    assert!(!is_manifest_url("https://cdn.example.com/index.m3u8/segment.ts"));
    assert!(!is_manifest_url(""));
}
