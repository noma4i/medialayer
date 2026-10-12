use super::leading_segments::STREAM_INF_TAG;

pub fn keep_first_variant(manifest: &str) -> String {
    if manifest.matches(STREAM_INF_TAG).count() < 2 {
        return manifest.to_string();
    }
    let mut seen = 0;
    let mut dropping = false;
    let mut kept = Vec::new();
    for line in manifest.split('\n') {
        let trimmed = line.trim();
        if trimmed.starts_with(STREAM_INF_TAG) {
            seen += 1;
            dropping = seen > 1;
        } else if dropping && !trimmed.starts_with('#') && !trimmed.is_empty() {
            dropping = false;
            continue;
        }
        if !dropping {
            kept.push(line);
        }
    }
    kept.join("\n")
}
