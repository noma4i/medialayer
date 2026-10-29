use super::import_extension;

#[test]
fn inv_import_05_extension_uses_name_then_mime() {
    for (name, mime, expected) in [
        (Some("picked.HEIC"), Some("image/jpeg"), "heic"),
        (Some("picked"), Some("video/quicktime"), "mov"),
        (Some("picked.///"), Some("image/png; charset=x"), "png"),
        (Some("picked."), Some(" VIDEO/MP4 "), "mp4"),
        (Some("picked.a-b"), Some("image/webp"), "webp"),
        (Some("picked.01234567890123456"), Some("image/avif"), "avif"),
        (None, Some("image/jpeg"), "jpg"),
        (None, Some("application/octet-stream"), ""),
        (None, None, ""),
    ] {
        assert_eq!(
            import_extension(name.map(str::to_owned), mime.map(str::to_owned)),
            expected
        );
    }
}
