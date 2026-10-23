use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SuspensionError {
    #[error("allow group is blank")]
    BlankGroup,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Suspensions {
    entries: BTreeMap<u64, Option<BTreeSet<String>>>,
}

impl Suspensions {
    pub fn set(&mut self, id: u64, is_active: bool, allow_groups: Option<&[&str]>) -> Result<bool, SuspensionError> {
        if !is_active {
            return Ok(self.entries.remove(&id).is_some());
        }
        let groups = allow_groups.map(normalize).transpose()?;
        Ok(self.entries.insert(id, groups.clone()) != Some(groups))
    }

    pub fn allows(&self, group: &str) -> bool {
        self.entries.values().all(|allowed| allowed.as_ref().is_some_and(|groups| groups.contains(group)))
    }

    pub fn allows_any(&self, groups: &BTreeSet<String>) -> bool {
        groups.iter().any(|group| self.allows(group))
    }
}

pub fn check_groups(groups: &[String]) -> Result<(), SuspensionError> {
    normalize(&groups.iter().map(String::as_str).collect::<Vec<_>>()).map(drop)
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
