use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub fn cache_namespace(url: &str, headers: &HashMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    append(&mut hasher, url);
    for (name, value) in normalized_headers(headers) {
        append(&mut hasher, &name);
        append(&mut hasher, &value);
    }
    super::hex_lower(&hasher.finalize())
}

fn normalized_headers(headers: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, value)| !name.is_empty() && !value.is_empty())
        .map(|(name, value)| (name.to_lowercase(), value.clone()))
        .collect();
    pairs.sort();
    pairs
}

fn append(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}
