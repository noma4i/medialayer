use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const NAMESPACE_HEADER: &str = "X-KTVHC-Cache-Namespace";
pub const FORWARDED_NAMES_HEADER: &str = "X-KTVHC-Forwarded-Header-Names";

pub fn cache_namespace(url: &str, headers: &HashMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    append(&mut hasher, url);
    for (name, value) in normalized_headers(headers) {
        append(&mut hasher, &name);
        append(&mut hasher, &value);
    }
    hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn transport_headers(url: &str, headers: &HashMap<String, String>) -> HashMap<String, String> {
    let source = source_headers(headers);
    let mut transport = source.clone();
    transport.insert(NAMESPACE_HEADER.to_string(), cache_namespace(url, &source));
    if !source.is_empty() {
        let mut names: Vec<String> = source.keys().map(|name| name.to_lowercase()).collect();
        names.sort();
        transport.insert(FORWARDED_NAMES_HEADER.to_string(), names.join(","));
    }
    transport
}

fn normalized_headers(headers: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = source_headers(headers)
        .into_iter()
        .filter(|(name, value)| !name.is_empty() && !value.is_empty())
        .map(|(name, value)| (name.to_lowercase(), value))
        .collect();
    pairs.sort();
    pairs
}

fn source_headers(headers: &HashMap<String, String>) -> HashMap<String, String> {
    headers
        .iter()
        .filter(|(name, _)| {
            !name.eq_ignore_ascii_case(NAMESPACE_HEADER) && !name.eq_ignore_ascii_case(FORWARDED_NAMES_HEADER)
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn append(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}
