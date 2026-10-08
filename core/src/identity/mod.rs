mod cache_key;
mod request_identity;

pub use cache_key::{cache_key, key_file_name, namespaced_key};
pub use request_identity::{cache_namespace, transport_headers, FORWARDED_NAMES_HEADER, NAMESPACE_HEADER};

#[cfg(test)]
mod cache_key_tests;
#[cfg(test)]
mod request_identity_tests;
