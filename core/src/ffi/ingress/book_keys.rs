use super::command::PlaceInput;
use crate::api::MediaItem;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Applied {
    pub(super) source: Option<MediaItem>,
    pub(super) playing: bool,
    pub(super) groups: Vec<Option<String>>,
    pub(super) peeking: bool,
    pub(super) muted: Option<bool>,
    pub(super) pinned: bool,
    pub(super) rate: Option<f32>,
    pub(super) token: Option<u64>,
}

#[derive(Default)]
pub(super) struct KeyOwners {
    pub(super) owners: usize,
    pub(super) owner_ids: BTreeSet<u64>,
    pub(super) visible: BTreeMap<u64, u64>,
    pub(super) playing: usize,
    pub(super) peeking: usize,
    pub(super) muted_some: usize,
    pub(super) muted_true: usize,
    pub(super) pins: usize,
    pub(super) groups: BTreeMap<Option<String>, usize>,
    pub(super) tokens: BTreeSet<(u8, u64, u64)>,
    pub(super) rates: BTreeSet<(u8, u64, u64)>,
    pub(super) applied: Applied,
}

impl KeyOwners {
    /// `None` while no owner has an opinion; otherwise muted only when every opinion asks for mute.
    pub(super) fn muted(&self) -> Option<bool> {
        (self.muted_some > 0).then_some(self.muted_true == self.muted_some)
    }

    pub(super) fn is_pinned(&self) -> bool {
        self.pins > 0
    }

    /// The owner whose view and rate win: playing first, then visible, then the newest.
    pub(super) fn token_owner(&self) -> Option<u64> {
        self.tokens.iter().next_back().map(|(_, _, owner)| *owner)
    }

    pub(super) fn rate_owner(&self) -> Option<u64> {
        self.rates.iter().next_back().map(|(_, _, owner)| *owner)
    }
}

pub(super) fn rank(input: &PlaceInput) -> u8 {
    u8::from(input.playing) * 2 + u8::from(input.visible)
}

pub(super) fn withdraw(keyed: &mut KeyOwners, owner: u64, input: &PlaceInput, declared: u64, intent: u64) {
    if declared != 0 {
        keyed.visible.remove(&declared);
    }
    if input.playing {
        keyed.playing -= 1;
        if let Some(count) = keyed.groups.get_mut(&input.group) {
            *count -= 1;
            if *count == 0 {
                keyed.groups.remove(&input.group);
            }
        }
    }
    keyed.peeking -= usize::from(input.peeking);
    if let Some(muted) = input.muted {
        keyed.muted_some -= 1;
        keyed.muted_true -= usize::from(muted);
    }
    keyed.pins -= usize::from(input.pinned);
    if input.view_token.is_some() {
        keyed.tokens.remove(&(rank(input), intent, owner));
    }
    if input.rate.is_some() {
        keyed.rates.remove(&(rank(input), intent, owner));
    }
}

pub(super) fn contribute(keyed: &mut KeyOwners, owner: u64, input: &PlaceInput, declared: u64, intent: u64) {
    if declared != 0 {
        keyed.visible.insert(declared, owner);
    }
    if input.playing {
        keyed.playing += 1;
        *keyed.groups.entry(input.group.clone()).or_default() += 1;
    }
    keyed.peeking += usize::from(input.peeking);
    if let Some(muted) = input.muted {
        keyed.muted_some += 1;
        keyed.muted_true += usize::from(muted);
    }
    keyed.pins += usize::from(input.pinned);
    if input.view_token.is_some() {
        keyed.tokens.insert((rank(input), intent, owner));
    }
    if input.rate.is_some() {
        keyed.rates.insert((rank(input), intent, owner));
    }
}
