#[uniffi::export]
pub fn import_extension(name: Option<String>, mime: Option<String>) -> String {
    if let Some(extension) = name
        .as_deref()
        .and_then(|name| name.rsplit_once('.').map(|(_, ext)| ext))
        .filter(|ext| {
            !ext.is_empty() && ext.len() <= 16 && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        })
    {
        return extension.to_ascii_lowercase();
    }
    match mime
        .as_deref()
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/heic" => "heic",
        "image/heif" => "heif",
        "image/avif" => "avif",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        _ => "",
    }
    .to_string()
}

#[cfg(test)]
mod import_tests;
