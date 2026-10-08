use sha2::{Digest, Sha256};

pub fn cache_key(url: &str) -> String {
    url.to_string()
}

pub fn key_file_name(key: &str) -> String {
    Sha256::digest(key.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
}
