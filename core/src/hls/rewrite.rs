const ABSOLUTE_PREFIX: &str = "http";
const REWRITE_PREFIX: &str = "./";

pub fn restore_rewritten(path: &str) -> String {
    match path.strip_prefix(REWRITE_PREFIX) {
        Some(rest) if rest.starts_with(ABSOLUTE_PREFIX) => rest.to_string(),
        _ => path.to_string(),
    }
}
