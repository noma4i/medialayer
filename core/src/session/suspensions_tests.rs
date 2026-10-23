use super::*;

#[test]
fn inv_session_03_any_active_reason_blocks_groups_it_does_not_allow() {
    let mut suspensions = Suspensions::default();
    assert_eq!(suspensions.set(1, true, None), Ok(true));
    assert!(!suspensions.allows("ring"));
    assert_eq!(suspensions.set(2, true, Some(&["hero"])), Ok(true));
    assert!(!suspensions.allows("hero"));
}

#[test]
fn inv_session_03_allow_groups_keep_listed_groups_eligible() {
    let mut suspensions = Suspensions::default();
    suspensions.set(2, true, Some(&["hero", "system"])).unwrap();
    assert!(suspensions.allows("hero"));
    assert!(suspensions.allows("system"));
    assert!(!suspensions.allows("ring"));
}

#[test]
fn inv_session_03_setting_the_same_value_again_changes_nothing() {
    let mut suspensions = Suspensions::default();
    assert_eq!(suspensions.set(3, true, Some(&["b", "a", "a"])), Ok(true));
    assert_eq!(suspensions.set(3, true, Some(&["a", "b"])), Ok(false));
    assert_eq!(suspensions.set(3, true, Some(&["a"])), Ok(true));
    assert_eq!(suspensions.set(3, false, None), Ok(true));
    assert_eq!(suspensions.set(3, false, None), Ok(false));
}

#[test]
fn inv_session_03_releasing_one_suspension_keeps_the_others() {
    let mut suspensions = Suspensions::default();
    suspensions.set(4, true, None).unwrap();
    suspensions.set(1, true, None).unwrap();
    assert_eq!(suspensions.set(4, false, None), Ok(true));
    assert_eq!(suspensions.set(4, false, None), Ok(false));
    assert!(!suspensions.allows("ring"));
    assert_eq!(suspensions.set(1, false, None), Ok(true));
    assert!(suspensions.allows("ring"));
}

#[test]
fn inv_session_03_blank_allow_group_is_rejected_and_changes_nothing() {
    let mut suspensions = Suspensions::default();
    assert_eq!(suspensions.set(3, true, Some(&["ring", "  "])), Err(SuspensionError::BlankGroup));
    assert!(suspensions.allows("ring"));
}

#[test]
fn inv_session_03_allow_groups_are_trimmed() {
    let mut suspensions = Suspensions::default();
    suspensions.set(3, true, Some(&[" b ", "a"])).unwrap();
    assert!(suspensions.allows("a"));
    assert!(suspensions.allows("b"));
    assert!(!suspensions.allows(" b "));
}
