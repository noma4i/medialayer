pub fn is_manifest_url(url: &str) -> bool {
    let without_fragment = url.split('#').next().unwrap_or(url);
    let without_query = without_fragment.split('?').next().unwrap_or(without_fragment);
    without_query.to_lowercase().ends_with(".m3u8")
}
