use super::positions::ResumePositions;

#[test]
fn inv_api_18_a_remembered_position_is_taken_once_and_only_the_newest_32_are_kept() {
    let mut positions = ResumePositions::default();
    for index in 0..40u64 {
        positions.remember(&format!("k{index}"), 1_000 + index);
    }

    assert_eq!(positions.take("k0"), None);
    assert_eq!(positions.take("k8"), Some(1_008));
    assert_eq!(positions.take("k8"), None);
    assert_eq!(positions.take("k39"), Some(1_039));
}
