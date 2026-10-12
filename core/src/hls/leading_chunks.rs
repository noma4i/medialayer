use url::Url;

pub(super) const STREAM_INF_TAG: &str = "#EXT-X-STREAM-INF";
const MAP_TAG: &str = "#EXT-X-MAP";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub segment: String,
    pub init: Option<String>,
}

pub fn is_multivariant(manifest: &str) -> bool {
    manifest.contains(STREAM_INF_TAG)
}

pub fn extract_variants(manifest: &str) -> Vec<String> {
    let lines: Vec<&str> = manifest.split('\n').map(str::trim).collect();
    let mut variants = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].starts_with(STREAM_INF_TAG) && index + 1 < lines.len() {
            let next = lines[index + 1];
            if !next.starts_with('#') && !next.is_empty() {
                variants.push(next.to_string());
            }
            index += 1;
        }
        index += 1;
    }
    variants
}

pub fn extract_leading_chunks(manifest: &str, max: usize) -> Vec<Chunk> {
    let mut current_init: Option<&str> = None;
    let mut chunks = Vec::new();
    for line in manifest.split('\n').map(str::trim) {
        if line.starts_with(MAP_TAG) {
            if let Some(uri) = quoted_uri(line) {
                current_init = Some(uri);
            }
        }
        if !line.starts_with('#') && !line.is_empty() {
            chunks.push(Chunk { segment: line.to_string(), init: current_init.map(str::to_string) });
            if chunks.len() == max {
                break;
            }
        }
    }
    chunks
}

pub fn resolve_url(base: &str, relative: &str) -> String {
    Url::parse(base)
        .and_then(|base_url| base_url.join(relative))
        .map(String::from)
        .unwrap_or_else(|_| relative.to_string())
}

fn quoted_uri(line: &str) -> Option<&str> {
    let (_, rest) = line.split_once("URI=\"")?;
    rest.split_once('"').map(|(uri, _)| uri)
}
