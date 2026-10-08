const ABSOLUTE_PREFIX: &str = "http";
const REWRITE_PREFIX: &str = "./";

pub fn rewrite_playlist(manifest: &str) -> String {
    if !manifest.contains("\nhttp") {
        return manifest.to_string();
    }
    manifest
        .split('\n')
        .map(|line| {
            if line.starts_with(ABSOLUTE_PREFIX) {
                format!("{REWRITE_PREFIX}{line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn restore_rewritten(path: &str) -> String {
    match path.strip_prefix(REWRITE_PREFIX) {
        Some(rest) if rest.starts_with(ABSOLUTE_PREFIX) => rest.to_string(),
        _ => path.to_string(),
    }
}
