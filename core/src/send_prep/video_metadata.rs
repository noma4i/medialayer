/// Keys that describe the container, not the person or the device. Every other key is private.
const TECHNICAL: [&str; 8] = ["major_brand", "minor_version", "compatible_brands", "handler_name", "vendor_id", "encoder", "©too", "tsse"];

/// The keys of a video file the result must not carry: location and GPS (`com.apple.quicktime.location.*`, `udta/©xyz`, `loci`), creation time, device and software, and every user tag. The platform reads the keys of the source and the core says which of them force a copy without metadata, an empty answer means the file is clean.
#[uniffi::export]
pub fn video_metadata_to_strip(keys: Vec<String>) -> Vec<String> {
    keys.into_iter()
        .filter(|key| {
            let name = key_name(key);
            !name.is_empty() && !TECHNICAL.contains(&name.as_str())
        })
        .collect()
}

fn key_name(key: &str) -> String {
    let lower = key.trim().to_lowercase();
    match lower.split_once('/') {
        Some((scheme, name)) if scheme.chars().count() <= 4 => name.to_string(),
        _ => lower,
    }
}
