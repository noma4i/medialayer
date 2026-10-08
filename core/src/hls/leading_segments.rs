use url::Url;

const STREAM_INF_TAG: &str = "#EXT-X-STREAM-INF";
const MAP_TAG: &str = "#EXT-X-MAP";
const EXTINF_TAG: &str = "#EXTINF";
const UNKNOWN_DURATION_SEGMENT_CAP: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LeadingSegments {
    pub init_segments: Vec<String>,
    pub segments: Vec<String>,
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

pub fn extract_leading_segments(manifest: &str, min_duration_seconds: f64) -> LeadingSegments {
    let mut current_init: Option<&str> = None;
    let mut pending_duration = None;
    let mut candidates: Vec<(&str, Option<f64>, Option<&str>)> = Vec::new();

    for line in manifest.split('\n').map(str::trim) {
        if line.starts_with(MAP_TAG) {
            if let Some(uri) = quoted_uri(line) {
                current_init = Some(uri);
            }
        }
        if line.starts_with(EXTINF_TAG) {
            pending_duration = extinf_duration(line);
            continue;
        }
        if !line.starts_with('#') && !line.is_empty() {
            candidates.push((line, pending_duration.take(), current_init));
        }
    }

    let limit = if candidates.iter().any(|(_, duration, _)| duration.is_none()) {
        UNKNOWN_DURATION_SEGMENT_CAP
    } else {
        candidates.len()
    };
    let mut segments = Vec::new();
    let mut init_segments: Vec<String> = Vec::new();
    let mut accumulated = 0.0;
    for (segment, duration, init) in candidates.into_iter().take(limit) {
        segments.push(segment.to_string());
        if let Some(init) = init.filter(|init| !init_segments.iter().any(|known| known == init)) {
            init_segments.push(init.to_string());
        }
        accumulated += duration.unwrap_or(0.0);
        if accumulated >= min_duration_seconds {
            break;
        }
    }
    LeadingSegments { init_segments, segments }
}

pub fn resolve_url(base: &str, relative: &str) -> String {
    Url::parse(base)
        .and_then(|base_url| base_url.join(relative))
        .map(String::from)
        .unwrap_or_else(|_| relative.to_string())
}

fn extinf_duration(line: &str) -> Option<f64> {
    let after_colon = line.split_once(':').map_or(line, |(_, rest)| rest);
    let value = after_colon.split(',').next()?.trim();
    value.parse::<f64>().ok().filter(|duration| duration.is_finite() && *duration >= 0.0)
}

fn quoted_uri(line: &str) -> Option<&str> {
    let (_, rest) = line.split_once("URI=\"")?;
    rest.split_once('"').map(|(uri, _)| uri)
}
