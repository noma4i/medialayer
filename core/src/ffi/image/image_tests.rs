use super::*;

const KNOWN: &str = "LEHV6nWB2yk8pyo0adR*.7kCMdnj";

#[test]
fn a_blurhash_decodes_to_rgba_bytes_and_a_bad_one_is_an_error_not_a_panic() {
    let pixels = decode_blurhash(KNOWN.to_string(), 32, 32, 1.0).unwrap();

    assert_eq!(pixels.len(), 32 * 32 * 4);
    assert!(matches!(decode_blurhash("short".to_string(), 32, 32, 1.0), Err(MediaError::BlurHash { .. })));
    assert!(matches!(decode_blurhash(KNOWN.to_string(), 0, 32, 1.0), Err(MediaError::BlurHash { .. })));
}
