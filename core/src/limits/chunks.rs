use super::settings::LimitsError;
use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 8;
const MAX_CHUNKS: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<u32>", into = "Vec<u32>")]
pub struct LeadingChunks {
    entries: [u8; MAX_ENTRIES],
    len: u8,
}

impl LeadingChunks {
    pub const MAX_CHUNKS: usize = MAX_CHUNKS as usize;

    pub fn new(values: &[u32]) -> Result<LeadingChunks, LimitsError> {
        let fail = |reason: &'static str| Err(LimitsError::LeadingChunks { reason });
        if values.is_empty() {
            return fail("must not be empty");
        }
        if values.len() > MAX_ENTRIES {
            return fail("has more than 8 entries");
        }
        if values.iter().any(|value| !(1..=MAX_CHUNKS).contains(value)) {
            return fail("entries must be between 1 and 8");
        }
        if values.windows(2).any(|pair| pair[1] > pair[0]) {
            return fail("must not grow with the distance");
        }
        let mut entries = [0; MAX_ENTRIES];
        for (slot, value) in entries.iter_mut().zip(values) {
            *slot = *value as u8;
        }
        Ok(LeadingChunks { entries, len: values.len() as u8 })
    }

    pub fn for_distance(&self, distance: usize) -> usize {
        let index = distance.clamp(1, self.len as usize) - 1;
        self.entries[index] as usize
    }

    pub fn to_vec(&self) -> Vec<u32> {
        self.entries[..self.len as usize].iter().map(|value| u32::from(*value)).collect()
    }
}

impl Default for LeadingChunks {
    fn default() -> Self {
        LeadingChunks::new(&[3, 2, 1]).expect("the default list is valid")
    }
}

impl TryFrom<Vec<u32>> for LeadingChunks {
    type Error = LimitsError;

    fn try_from(values: Vec<u32>) -> Result<Self, Self::Error> {
        LeadingChunks::new(&values)
    }
}

impl From<LeadingChunks> for Vec<u32> {
    fn from(chunks: LeadingChunks) -> Vec<u32> {
        chunks.to_vec()
    }
}
