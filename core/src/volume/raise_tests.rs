use super::*;

#[test]
fn inv_audio_09_only_a_rise_above_the_threshold_counts() {
    assert!(is_volume_raise(0.5, 0.7));
    assert!(is_volume_raise(0.0, 0.002));
    assert!(!is_volume_raise(0.5, 0.5));
    assert!(!is_volume_raise(0.5, 0.5005));
    assert!(!is_volume_raise(0.5, 0.3));
    assert!(!is_volume_raise(1.0, 0.0));
}
