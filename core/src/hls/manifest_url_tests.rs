use super::*;

#[test]
fn inv_hls_04_only_the_path_extension_marks_a_manifest_url() {
    for url in [
        "https://cdn.example.com/a/index.m3u8",
        "https://cdn.example.com/a/INDEX.M3U8",
        "https://cdn.example.com/index.m3u8?token=1",
        "https://cdn.example.com/index.m3u8#t=3",
        "https://cdn.example.com/index.m3u8?a=b#frag",
    ] {
        assert!(is_manifest_url(url), "{url}");
    }
    for url in [
        "https://cdn.example.com/video.mp4",
        "https://cdn.example.com/video.mp4?src=index.m3u8",
        "https://cdn.example.com/index.m3u8/segment.ts",
        "",
    ] {
        assert!(!is_manifest_url(url), "{url}");
    }
}
