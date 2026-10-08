use super::*;

#[test]
fn prefixes_absolute_lines_with_dot_slash() {
    let manifest = "#EXTM3U\n#EXTINF:2.0,\nhttps://cdn.example.com/a.ts\n";
    assert_eq!(
        rewrite_playlist(manifest),
        "#EXTM3U\n#EXTINF:2.0,\n./https://cdn.example.com/a.ts\n"
    );
}

#[test]
fn keeps_relative_lines_and_tags() {
    let manifest = "#EXTM3U\n#EXTINF:2.0,\na.ts\n#EXTINF:2.0,\nhttp://h/b.ts\n";
    assert_eq!(
        rewrite_playlist(manifest),
        "#EXTM3U\n#EXTINF:2.0,\na.ts\n#EXTINF:2.0,\n./http://h/b.ts\n"
    );
}

#[test]
fn leaves_a_playlist_without_absolute_lines_unchanged() {
    let manifest = "#EXTM3U\n#EXTINF:2.0,\na.ts\n";
    assert_eq!(rewrite_playlist(manifest), manifest);
    assert_eq!(rewrite_playlist(""), "");
}

#[test]
fn ignores_an_absolute_url_on_the_first_line() {
    let manifest = "http://h/a.ts\n#EXTM3U\n";
    assert_eq!(rewrite_playlist(manifest), manifest);
}

#[test]
fn keeps_crlf_endings_and_blank_lines() {
    let manifest = "#EXTM3U\r\n\r\n#EXTINF:2.0,\r\nhttp://h/a.ts\r\n\r\n";
    assert_eq!(
        rewrite_playlist(manifest),
        "#EXTM3U\r\n\r\n#EXTINF:2.0,\r\n./http://h/a.ts\r\n\r\n"
    );
}

#[test]
fn rewrites_map_uri_in_quotes() {
    let manifest = "#EXTM3U\n#EXT-X-MAP:URI=\"http://h/init.mp4\"\nhttp://h/a.m4s\n";
    assert_eq!(
        rewrite_playlist(manifest),
        "#EXTM3U\n#EXT-X-MAP:URI=\"./http://h/init.mp4\"\n./http://h/a.m4s\n"
    );
}

#[test]
fn keeps_parent_relative_paths_and_segment_queries() {
    let manifest = "#EXTM3U\n../seg/a.ts?token=1\nhttps://h/b.ts?x=1&y=2\n";
    assert_eq!(
        rewrite_playlist(manifest),
        "#EXTM3U\n../seg/a.ts?token=1\n./https://h/b.ts?x=1&y=2\n"
    );
}

#[test]
fn restore_strips_the_prefix_from_absolute_paths() {
    assert_eq!(restore_rewritten("./https://h/a.ts?x=1"), "https://h/a.ts?x=1");
    assert_eq!(restore_rewritten("./http://h/a.ts"), "http://h/a.ts");
}

#[test]
fn restore_keeps_other_paths() {
    assert_eq!(restore_rewritten("a.ts"), "a.ts");
    assert_eq!(restore_rewritten("./a.ts"), "./a.ts");
    assert_eq!(restore_rewritten("../a.ts"), "../a.ts");
    assert_eq!(restore_rewritten(""), "");
}

#[test]
fn restore_inverts_rewrite_for_each_line() {
    let manifest = "#EXTM3U\nhttp://h/a.ts\nb.ts\nhttps://h/c.ts?q=1\n";
    let restored: Vec<String> = rewrite_playlist(manifest).split('\n').map(restore_rewritten).collect();
    assert_eq!(restored.join("\n"), manifest);
}

#[test]
fn prefixes_absolute_uris_inside_tags() {
    let manifest = "#EXTM3U\n#EXT-X-MAP:URI=\"https://cdn/init.mp4\",BYTERANGE=\"720@0\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"http://k/key.bin\",IV=0x01\n#EXTINF:2.0,\na.m4s\n";
    assert_eq!(
        rewrite_playlist(manifest),
        "#EXTM3U\n#EXT-X-MAP:URI=\"./https://cdn/init.mp4\",BYTERANGE=\"720@0\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"./http://k/key.bin\",IV=0x01\n#EXTINF:2.0,\na.m4s\n"
    );
}

#[test]
fn rewrites_media_and_iframe_tags_with_uris() {
    let manifest = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"https://cdn/a.m3u8\"\n#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=1,URI=\"https://cdn/i.m3u8\"\n";
    assert_eq!(
        rewrite_playlist(manifest),
        "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"./https://cdn/a.m3u8\"\n#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=1,URI=\"./https://cdn/i.m3u8\"\n"
    );
}

#[test]
fn leaves_relative_and_non_http_tag_uris_alone() {
    let manifest = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key\"\n";
    assert_eq!(rewrite_playlist(manifest), manifest);
}
