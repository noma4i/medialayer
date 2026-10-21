use super::*;

#[test]
fn inv_hls_03_absolute_lines_get_a_dot_slash_prefix_and_everything_else_is_kept() {
    let cases = [
        ("absolute line", "#EXTM3U\n#EXTINF:2.0,\nhttps://cdn.example.com/a.ts\n", "#EXTM3U\n#EXTINF:2.0,\n./https://cdn.example.com/a.ts\n"),
        ("relative lines and tags", "#EXTM3U\n#EXTINF:2.0,\na.ts\n#EXTINF:2.0,\nhttp://h/b.ts\n", "#EXTM3U\n#EXTINF:2.0,\na.ts\n#EXTINF:2.0,\n./http://h/b.ts\n"),
        ("no absolute lines", "#EXTM3U\n#EXTINF:2.0,\na.ts\n", "#EXTM3U\n#EXTINF:2.0,\na.ts\n"),
        ("empty playlist", "", ""),
        ("absolute url on the first line", "http://h/a.ts\n#EXTM3U\n", "./http://h/a.ts\n#EXTM3U\n"),
        ("crlf and blank lines", "#EXTM3U\r\n\r\n#EXTINF:2.0,\r\nhttp://h/a.ts\r\n\r\n", "#EXTM3U\r\n\r\n#EXTINF:2.0,\r\n./http://h/a.ts\r\n\r\n"),
        ("parent paths and queries", "#EXTM3U\n../seg/a.ts?token=1\nhttps://h/b.ts?x=1&y=2\n", "#EXTM3U\n../seg/a.ts?token=1\n./https://h/b.ts?x=1&y=2\n"),
    ];
    for (name, manifest, expected) in cases {
        assert_eq!(String::from_utf8(Playlist::parse(manifest, "").body.to_vec()).unwrap(), expected, "{name}");
    }
}

#[test]
fn inv_hls_03_absolute_http_uris_inside_tags_get_the_prefix_and_other_tag_uris_are_kept() {
    let cases = [
        (
            "map, key",
            "#EXTM3U\n#EXT-X-MAP:URI=\"https://cdn/init.mp4\",BYTERANGE=\"720@0\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"http://k/key.bin\",IV=0x01\n#EXTINF:2.0,\na.m4s\n",
            "#EXTM3U\n#EXT-X-MAP:URI=\"./https://cdn/init.mp4\",BYTERANGE=\"720@0\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"./http://k/key.bin\",IV=0x01\n#EXTINF:2.0,\na.m4s\n",
        ),
        (
            "media, iframe",
            "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"https://cdn/a.m3u8\"\n#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=1,URI=\"https://cdn/i.m3u8\"\n",
            "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,URI=\"./https://cdn/a.m3u8\"\n#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=1,URI=\"./https://cdn/i.m3u8\"\n",
        ),
        (
            "map uri followed by an absolute segment",
            "#EXTM3U\n#EXT-X-MAP:URI=\"http://h/init.mp4\"\nhttp://h/a.m4s\n",
            "#EXTM3U\n#EXT-X-MAP:URI=\"./http://h/init.mp4\"\n./http://h/a.m4s\n",
        ),
        (
            "relative and non-http uris",
            "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key\"\n",
            "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://key\"\n",
        ),
    ];
    for (name, manifest, expected) in cases {
        assert_eq!(String::from_utf8(Playlist::parse(manifest, "").body.to_vec()).unwrap(), expected, "{name}");
    }
}

#[test]
fn inv_hls_03_restore_strips_the_prefix_from_absolute_paths_only() {
    let cases = [
        ("./https://h/a.ts?x=1", "https://h/a.ts?x=1"),
        ("./http://h/a.ts", "http://h/a.ts"),
        ("a.ts", "a.ts"),
        ("./a.ts", "./a.ts"),
        ("../a.ts", "../a.ts"),
        ("", ""),
    ];
    for (path, expected) in cases {
        assert_eq!(restore_rewritten(path), expected, "{path}");
    }
}

#[test]
fn inv_hls_03_restore_inverts_rewrite_for_each_line() {
    let manifest = "#EXTM3U\nhttp://h/a.ts\nb.ts\nhttps://h/c.ts?q=1\n";
    let restored: Vec<String> = String::from_utf8(Playlist::parse(manifest, "").body.to_vec()).unwrap().split('\n').map(restore_rewritten).collect();
    assert_eq!(restored.join("\n"), manifest);
}
