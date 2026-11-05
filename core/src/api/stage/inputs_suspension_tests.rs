use crate::api::slots::SlotCall;
use crate::api::test_support::{eventually, rig, url_of, videos};

fn transport(rig: &crate::api::test_support::Rig, key: &str) -> Option<SlotCall> {
    rig.factory.slot_holding(key)?.1.calls().into_iter().rfind(|call| matches!(call, SlotCall::Play | SlotCall::Pause))
}

#[tokio::test]
async fn inv_session_12_players_stay_paused_until_the_last_suspension_is_released() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &[]);
    eventually(|| rig.factory.slot_holding(&url_of(5)).is_some_and(|(_, slot)| slot.calls().contains(&SlotCall::Play))).await;

    stage.suspend(1, &[]);
    stage.suspend(2, &[]);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Pause)).await;

    stage.resume(1);
    eventually(|| stage.commands_idle()).await;
    assert_eq!(transport(&rig, &url_of(5)), Some(SlotCall::Pause), "the first release resumed the player");

    stage.resume(2);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Play)).await;
}

#[tokio::test]
async fn inv_session_12_only_a_group_every_suspension_allows_keeps_playing() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &[]);
    stage.set_playing(&url_of(6), true, &[Some("chat".to_string())]);
    eventually(|| transport(&rig, &url_of(6)) == Some(SlotCall::Play)).await;

    stage.suspend(1, &["chat".to_string(), "voice".to_string()]);
    stage.suspend(2, &["chat".to_string()]);
    eventually(|| stage.commands_idle()).await;
    assert_eq!(transport(&rig, &url_of(6)), Some(SlotCall::Play), "a group allowed by every suspension stopped");

    stage.suspend(3, &["voice".to_string()]);
    eventually(|| transport(&rig, &url_of(6)) == Some(SlotCall::Pause)).await;
    stage.resume(3);
    eventually(|| transport(&rig, &url_of(6)) == Some(SlotCall::Play)).await;

    stage.set_playing(&url_of(6), true, &[]);
    eventually(|| transport(&rig, &url_of(6)) == Some(SlotCall::Pause)).await;
}

#[tokio::test]
async fn inv_session_14_a_key_with_several_groups_plays_if_one_group_is_allowed_by_every_suspension() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &[Some("ring".to_string()), Some("system".to_string())]);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Play)).await;

    stage.suspend(1, &["system".to_string()]);
    eventually(|| stage.commands_idle()).await;
    assert_eq!(transport(&rig, &url_of(5)), Some(SlotCall::Play), "a group allowed by the only suspension stopped");

    stage.suspend(2, &["ring".to_string()]);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Pause)).await;
    stage.resume(2);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Play)).await;
}

fn ungrouped_and_system() -> [Option<String>; 2] {
    [None, Some("system".to_string())]
}

#[tokio::test]
async fn inv_session_17_an_ungrouped_owner_counts_as_the_default_group_under_a_suspension() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &ungrouped_and_system());
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Play)).await;

    stage.suspend(1, &["ring".to_string()]);
    eventually(|| stage.commands_idle()).await;
    assert_eq!(transport(&rig, &url_of(5)), Some(SlotCall::Play), "an ungrouped owner is in the default group and was paused");
}

#[tokio::test]
async fn inv_session_17_suspensions_for_the_default_group_and_system_pause_ungrouped_and_system_owners() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &ungrouped_and_system());
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Play)).await;

    stage.suspend(1, &["ring".to_string()]);
    stage.suspend(2, &["system".to_string()]);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Pause)).await;
}

#[tokio::test]
async fn inv_session_17_the_default_group_leaves_the_set_when_the_ungrouped_owner_leaves() {
    let rig = rig().await;
    let stage = &rig.parts.stage;
    stage.replace_window(5, videos(20));
    stage.set_playing(&url_of(5), true, &ungrouped_and_system());
    stage.suspend(1, &["ring".to_string()]);
    eventually(|| stage.commands_idle()).await;
    assert_eq!(transport(&rig, &url_of(5)), Some(SlotCall::Play));

    stage.set_playing(&url_of(5), true, &[Some("system".to_string())]);
    eventually(|| transport(&rig, &url_of(5)) == Some(SlotCall::Pause)).await;
}
