use super::*;

#[test]
fn the_url_is_the_key() {
    assert_eq!(cache_key("https://cdn.example.com/A/b.m3u8?X=1"), "https://cdn.example.com/A/b.m3u8?X=1");
}

#[test]
fn file_name_of_a_key_is_stable_hex() {
    let name = key_file_name("https://a.example.com/x");
    assert_eq!(name.len(), 64);
    assert!(name.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(name, key_file_name("https://a.example.com/x"));
}
