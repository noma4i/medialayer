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

pub fn hex_lower(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    hex_into(bytes, &mut text);
    text
}

pub fn hex_into(bytes: &[u8], text: &mut String) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        text.push(DIGITS[usize::from(byte >> 4)] as char);
        text.push(DIGITS[usize::from(byte & 15)] as char);
    }
}

pub fn key_file_name(key: &str) -> String {
    hex_lower(&Sha256::digest(key.as_bytes()))
}
