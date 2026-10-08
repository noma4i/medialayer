use super::*;

#[test]
fn a_length_at_the_limit_passes_and_one_more_fails() {
    assert_eq!(bounded(10, 10), Ok(()));
    assert_eq!(bounded(11, 10), Err(TooLarge { len: 11, limit: 10 }));
}

#[test]
fn zero_length_passes() {
    assert_eq!(bounded(0, 0), Ok(()));
}

#[test]
fn default_limits_reject_inputs_beyond_each_cap() {
    let limits = InputLimits::default();
    assert!(bounded(limits.playlist_bytes, limits.playlist_bytes).is_ok());
    assert!(bounded(limits.playlist_bytes + 1, limits.playlist_bytes).is_err());
    assert!(bounded(limits.response_bytes + 1, limits.response_bytes).is_err());
    assert!(bounded(limits.blurhash_chars + 1, limits.blurhash_chars).is_err());
}

#[test]
fn a_valid_blurhash_fits_the_default_cap() {
    assert!(InputLimits::default().blurhash_chars >= 4 + 2 * 9 * 9);
}

#[test]
fn an_oversized_playlist_is_rejected_with_its_size() {
    let limits = InputLimits::default();
    let error = bounded(50 * 1024 * 1024, limits.playlist_bytes).unwrap_err();
    assert_eq!(error.len, 50 * 1024 * 1024);
    assert_eq!(error.limit, limits.playlist_bytes);
}
