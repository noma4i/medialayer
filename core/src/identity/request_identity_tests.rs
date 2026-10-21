use super::*;
use std::collections::HashMap;

const URL: &str = "https://cdn.example.com/v/index.m3u8";

#[test]
fn inv_proxy_15_namespace_uses_one_digest_allocation() {
    for headers in [map(&[]), map(&[("Authorization", "a")])] {
        let allocations = crate::alloc_probe::allocations_of(|| cache_namespace(URL, &headers));
        assert!(allocations <= 4, "namespace allocated {allocations} times");
    }
}

fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn inv_proxy_15_namespace_matches_the_independent_sha256_vectors() {
    let cases = [
        ("bare", URL.to_string(), map(&[]), "982329f070ea6cf08370524b340e445e6df6a3941c5ea34114812ada87da11e3"),
        ("two headers", URL.to_string(), map(&[("Authorization", "a"), ("X-Token", "b")]), "0bfbd247ee451169591cd8c74cdf32edf8666b73fd6d557446c5605523062faa"),
        ("header value a", URL.to_string(), map(&[("Authorization", "a")]), "c33f2619d7608b81d686cc01e55414231f27e10e789377928d1c6243419696f9"),
        ("header value b", URL.to_string(), map(&[("Authorization", "b")]), "61acf35fb8377732c0261686273723e2e7dda24e21577ce8bc4a3857c5a5a165"),
        ("other url", format!("{URL}?x=1"), map(&[("Authorization", "a")]), "95c46982423094b8e229f8aad43cca4b4bd007aca8346cf226c5e1bcd74c03b0"),
        ("length prefix ab/c", URL.to_string(), map(&[("ab", "c")]), "27b49d1b09a4d1fe018bb5d05574dec53076ae760e1c51e0e4f99b358fdc73f5"),
        ("length prefix a/bc", URL.to_string(), map(&[("a", "bc")]), "38e0e23f326eb343ba24105625aaee3c5cf124539ba6f8a8bc5cd060268daa0a"),
    ];
    for (name, url, headers, expected) in cases {
        assert_eq!(cache_namespace(&url, &headers), expected, "{name}");
    }
}

#[test]
fn inv_proxy_15_namespace_ignores_case_and_order() {
    let first = cache_namespace(URL, &map(&[("Authorization", "a"), ("X-Token", "b")]));
    let second = cache_namespace(URL, &map(&[("x-token", "b"), ("authorization", "a")]));
    assert_eq!(first, second);
}

#[test]
fn inv_proxy_15_namespace_drops_empty_name_and_empty_value() {
    let bare = cache_namespace(URL, &map(&[]));
    assert_eq!(cache_namespace(URL, &map(&[("X-Empty", "")])), bare);
    assert_eq!(cache_namespace(URL, &map(&[("", "value")])), bare);
}
