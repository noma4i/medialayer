use super::*;
use crate::limits::Limits;
use std::collections::HashMap;
use std::time::Duration;

fn headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect()
}

#[test]
fn default_timeout_comes_from_limits() {
    assert_eq!(FetchPolicy::default().timeout, Duration::from_secs(30));
    let limits = Limits { timeout_seconds: 7, ..Limits::default() };
    assert_eq!(FetchPolicy::from_limits(&limits).timeout, Duration::from_secs(7));
}

#[test]
fn default_whitelist_forwards_the_eight_v3_headers_and_drops_the_rest() {
    let input = headers(&[
        ("Authorization", "Bearer t"),
        ("cookie", "a=b"),
        ("User-Agent", "ua"),
        ("Connection", "keep-alive"),
        ("Accept", "*/*"),
        ("Accept-Encoding", "gzip"),
        ("Accept-Language", "en"),
        ("Range", "bytes=0-1"),
        ("X-Custom", "no"),
        ("Host", "localhost"),
    ]);
    let forwarded = FetchPolicy::default().forward(&input);
    assert_eq!(forwarded.len(), 8);
    assert!(!forwarded.contains_key("X-Custom"));
    assert!(!forwarded.contains_key("Host"));
    assert_eq!(forwarded.get("cookie").map(String::as_str), Some("a=b"));
}

#[test]
fn whitelist_is_configurable() {
    let policy = FetchPolicy::default().with_forwarded_headers(&["X-Custom"]);
    let forwarded = policy.forward(&headers(&[("x-custom", "1"), ("Authorization", "t")]));
    assert_eq!(forwarded, headers(&[("x-custom", "1")]));
}

#[test]
fn header_lookup_ignores_case() {
    let input = headers(&[("Content-Length", "5")]);
    assert_eq!(header_value(&input, "content-length"), Some("5"));
    assert_eq!(header_value(&input, "etag"), None);
}
