use super::*;

#[test]
fn the_url_is_the_key() {
    assert_eq!(cache_key("https://cdn.example.com/a/b.m3u8?x=1"), "https://cdn.example.com/a/b.m3u8?x=1");
}

#[test]
fn different_urls_give_different_keys() {
    assert_ne!(cache_key("https://a.example.com/x"), cache_key("https://a.example.com/y"));
}

#[test]
fn file_name_of_a_key_is_stable_hex() {
    let name = key_file_name("https://a.example.com/x");
    assert_eq!(name.len(), 64);
    assert!(name.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(name, key_file_name("https://a.example.com/x"));
}
