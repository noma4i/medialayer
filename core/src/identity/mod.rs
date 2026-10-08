mod request_identity;

pub use request_identity::{cache_namespace, transport_headers, FORWARDED_NAMES_HEADER, NAMESPACE_HEADER};

#[cfg(test)]
mod request_identity_tests;
