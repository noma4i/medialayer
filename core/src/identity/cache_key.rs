use super::request_identity::cache_namespace;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const NAMESPACE_MARK: &str = "#ns:";

pub fn cache_key(url: &str) -> String {
    url.to_string()
}

pub fn namespaced_key(url: &str, headers: &HashMap<String, String>, names: &[String]) -> String {
    let selected: HashMap<String, String> = headers
        .iter()
        .filter(|(name, value)| !value.is_empty() && names.iter().any(|listed| listed.eq_ignore_ascii_case(name)))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    if selected.is_empty() {
        return cache_key(url);
    }
    format!("{}{NAMESPACE_MARK}{}", cache_key(url), cache_namespace(url, &selected))
}

pub fn key_file_name(key: &str) -> String {
    Sha256::digest(key.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}
