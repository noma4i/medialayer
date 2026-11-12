use super::*;

#[test]
fn inv_proxy_15_file_name_of_a_key_is_stable_hex() {
    let name = key_file_name("https://a.example.com/x");
    assert_eq!(name.len(), 64);
    assert!(name.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(name, key_file_name("https://a.example.com/x"));
}

fn headers(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
    pairs.iter().map(|(name, value)| (name.to_string(), value.to_string())).collect()
}

#[test]
fn inv_proxy_15_the_key_follows_only_the_listed_headers_that_carry_a_value() {
    let url = "https://cdn.example.com/a.mp4";
    let listed = vec!["authorization".to_string()];
    let a = namespaced_key(url, &headers(&[("Authorization", "a")]), &listed);
    let b = namespaced_key(url, &headers(&[("Authorization", "b")]), &listed);

    assert_ne!(a, b);
    assert!(a.starts_with(url));
    assert_eq!(a, namespaced_key(url, &headers(&[("AUTHORIZATION", "a"), ("Accept", "*/*")]), &listed));
    let plain = [
        ("no listed names", namespaced_key(url, &headers(&[("Authorization", "a")]), &[])),
        ("header absent", namespaced_key(url, &headers(&[("Accept", "*/*")]), &listed)),
        ("header empty", namespaced_key(url, &headers(&[("Authorization", "")]), &listed)),
    ];
    for (name, key) in plain {
        assert_eq!(key, url, "{name}");
    }
}
