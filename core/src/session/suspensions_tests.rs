use super::*;

#[test]
fn any_active_reason_blocks_groups_it_does_not_allow() {
    let mut suspensions = Suspensions::default();
    assert_eq!(suspensions.set(Reason::Interruption, true, None), Ok(true));
    assert!(!suspensions.allows("ring"));
    assert_eq!(suspensions.set(Reason::AudioFocusTransient, true, Some(&["hero"])), Ok(true));
    assert!(!suspensions.allows("hero"));
}

#[test]
fn allow_groups_keep_listed_groups_eligible() {
    let mut suspensions = Suspensions::default();
    suspensions.set(Reason::AudioFocusTransient, true, Some(&["hero", "system"])).unwrap();
    assert!(suspensions.allows("hero"));
    assert!(suspensions.allows("system"));
    assert!(!suspensions.allows("ring"));
}

#[test]
fn setting_the_same_value_again_changes_nothing() {
    let mut suspensions = Suspensions::default();
    assert_eq!(suspensions.set(Reason::Manual, true, Some(&["b", "a", "a"])), Ok(true));
    assert_eq!(suspensions.set(Reason::Manual, true, Some(&["a", "b"])), Ok(false));
    assert_eq!(suspensions.set(Reason::Manual, true, Some(&["a"])), Ok(true));
    assert_eq!(suspensions.set(Reason::Manual, false, None), Ok(true));
    assert_eq!(suspensions.set(Reason::Manual, false, None), Ok(false));
}

#[test]
fn thaw_removes_only_its_reason() {
    let mut suspensions = Suspensions::default();
    suspensions.set(Reason::Background, true, None).unwrap();
    suspensions.set(Reason::Interruption, true, None).unwrap();
    assert!(suspensions.thaw(Reason::Background));
    assert!(!suspensions.thaw(Reason::Background));
    assert!(!suspensions.allows("ring"));
    assert!(suspensions.thaw_all());
    assert!(!suspensions.thaw_all());
    assert!(suspensions.allows("ring"));
}

#[test]
fn blank_allow_group_is_rejected_and_changes_nothing() {
    let mut suspensions = Suspensions::default();
    assert_eq!(suspensions.set(Reason::Manual, true, Some(&["ring", "  "])), Err(SuspensionError::BlankGroup));
    assert!(suspensions.is_empty());
}

#[test]
fn allow_groups_are_trimmed_and_snapshot_is_sorted_by_reason() {
    let mut suspensions = Suspensions::default();
    suspensions.set(Reason::Manual, true, Some(&[" b ", "a"])).unwrap();
    suspensions.set(Reason::AudioFocusTransient, true, None).unwrap();
    assert_eq!(
        suspensions.snapshot(),
        [(Reason::AudioFocusTransient, None), (Reason::Manual, Some(vec!["a".to_string(), "b".to_string()]))]
    );
}
