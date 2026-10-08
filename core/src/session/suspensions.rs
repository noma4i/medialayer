use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    AudioFocusTransient,
    Interruption,
    Background,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SuspensionError {
    #[error("allow group is blank")]
    BlankGroup,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Suspensions {
    entries: BTreeMap<Reason, Option<BTreeSet<String>>>,
}

impl Suspensions {
    pub fn set(&mut self, reason: Reason, is_active: bool, allow_groups: Option<&[&str]>) -> Result<bool, SuspensionError> {
        if !is_active {
            return Ok(self.entries.remove(&reason).is_some());
        }
        let groups = allow_groups.map(normalize).transpose()?;
        Ok(self.entries.insert(reason, groups.clone()) != Some(groups))
    }

    pub fn thaw(&mut self, reason: Reason) -> bool {
        self.entries.remove(&reason).is_some()
    }

    pub fn thaw_all(&mut self) -> bool {
        let had_entries = !self.entries.is_empty();
        self.entries.clear();
        had_entries
    }

    pub fn allows(&self, group: &str) -> bool {
        self.entries.values().all(|allowed| allowed.as_ref().is_some_and(|groups| groups.contains(group)))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn snapshot(&self) -> Vec<(Reason, Option<Vec<String>>)> {
        self.entries.iter().map(|(reason, groups)| (*reason, groups.as_ref().map(|groups| groups.iter().cloned().collect()))).collect()
    }
}

fn normalize(groups: &[&str]) -> Result<BTreeSet<String>, SuspensionError> {
    groups
        .iter()
        .map(|group| match group.trim() {
            "" => Err(SuspensionError::BlankGroup),
            trimmed => Ok(trimmed.to_string()),
        })
        .collect()
}
