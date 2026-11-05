use super::book_keys::*;
use super::command::PlaceInput;

fn input(visible: bool, playing: bool, group: Option<&str>) -> PlaceInput {
    PlaceInput { visible, playing, group: group.map(str::to_string), ..PlaceInput::default() }
}

fn join(keyed: &mut KeyOwners, owner: u64, input: &PlaceInput, intent: u64) {
    contribute(keyed, owner, input, if input.visible { intent } else { 0 }, intent);
}

#[test]
fn inv_ingress_22_visible_and_peeking_are_or_and_the_source_returns_to_the_last_remaining_declaration() {
    let mut keyed = KeyOwners::default();
    let (first, second) = (input(true, false, None), PlaceInput { visible: true, peeking: true, ..PlaceInput::default() });
    join(&mut keyed, 1, &first, 1);
    join(&mut keyed, 2, &second, 2);
    assert_eq!(keyed.visible.iter().next_back().map(|(_, owner)| *owner), Some(2));

    withdraw(&mut keyed, 2, &second, 2, 2);

    assert_eq!(keyed.visible.iter().next_back().map(|(_, owner)| *owner), Some(1));
    assert_eq!(keyed.peeking, 0);
    withdraw(&mut keyed, 1, &first, 1, 1);
    assert!(keyed.visible.is_empty());
}

#[test]
fn inv_ingress_23_playing_is_or_and_groups_are_the_set_of_the_playing_owners() {
    let mut keyed = KeyOwners::default();
    let (chat, voice, idle) = (input(false, true, Some("chat")), input(false, true, Some("voice")), input(false, false, Some("idle")));
    join(&mut keyed, 1, &chat, 1);
    join(&mut keyed, 2, &voice, 2);
    join(&mut keyed, 3, &idle, 3);
    assert_eq!((keyed.playing, keyed.groups.keys().cloned().collect::<Vec<_>>()), (2, vec![Some("chat".to_string()), Some("voice".to_string())]));

    withdraw(&mut keyed, 2, &voice, 0, 2);

    assert_eq!((keyed.playing, keyed.groups.keys().cloned().collect::<Vec<_>>()), (1, vec![Some("chat".to_string())]));
    let ungrouped = input(false, true, None);
    join(&mut keyed, 4, &ungrouped, 4);
    assert_eq!(keyed.groups.keys().cloned().collect::<Vec<_>>(), vec![None, Some("chat".to_string())]);
}

#[test]
fn inv_ingress_24_a_key_is_muted_only_while_every_opinion_asks_for_mute() {
    let mut keyed = KeyOwners::default();
    let asking = PlaceInput { muted: Some(true), ..PlaceInput::default() };
    let refusing = PlaceInput { muted: Some(false), ..PlaceInput::default() };
    join(&mut keyed, 1, &asking, 1);
    join(&mut keyed, 2, &refusing, 2);
    assert_eq!(keyed.muted(), Some(false));

    withdraw(&mut keyed, 2, &refusing, 0, 2);
    assert_eq!(keyed.muted(), Some(true));
    join(&mut keyed, 2, &asking, 3);

    assert_eq!(keyed.muted(), Some(true));
    withdraw(&mut keyed, 1, &asking, 0, 1);
    withdraw(&mut keyed, 2, &asking, 0, 3);
    assert_eq!(keyed.muted(), None);
}

#[test]
fn inv_ingress_25_the_view_and_the_rate_come_from_the_best_live_owner_playing_first_then_visible_then_newest() {
    let mut keyed = KeyOwners::default();
    let shown = PlaceInput { visible: true, view_token: Some(1), rate: Some(1.5), ..PlaceInput::default() };
    let playing = PlaceInput { playing: true, view_token: Some(2), rate: Some(2.0), ..PlaceInput::default() };
    let newer = PlaceInput { view_token: Some(3), ..PlaceInput::default() };
    join(&mut keyed, 1, &shown, 1);
    join(&mut keyed, 2, &playing, 2);
    join(&mut keyed, 3, &newer, 3);
    assert_eq!(keyed.token_owner(), Some(2));
    assert_eq!(keyed.rate_owner(), Some(2));

    withdraw(&mut keyed, 2, &playing, 0, 2);

    assert_eq!(keyed.token_owner(), Some(1));
    assert_eq!(keyed.rate_owner(), Some(1));
}

#[test]
fn inv_ingress_26_a_pin_lives_while_any_owner_pins() {
    let mut keyed = KeyOwners::default();
    let pinned = PlaceInput { pinned: true, ..PlaceInput::default() };
    join(&mut keyed, 1, &pinned, 1);
    join(&mut keyed, 2, &pinned, 2);

    withdraw(&mut keyed, 1, &pinned, 0, 1);
    assert!(keyed.is_pinned());
    withdraw(&mut keyed, 2, &pinned, 0, 2);

    assert!(!keyed.is_pinned());
}
