use crate::hls::{resolve_url, restore_rewritten};
use url::{form_urlencoded, Url};

const QUERY_ROUTE_PREFIX: &str = "/request.";
const QUERY_PARAMETER: &str = "url";
const TAG_URI_OPEN: &str = "URI=\"";
const FALLBACK_EXTENSION: &str = "bin";
const MAX_EXTENSION_LENGTH: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRange {
    From { start: u64, end: Option<u64> },
    Suffix(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeOutcome {
    Full,
    Partial { start: u64, end: u64 },
    Unsatisfiable,
}

pub fn proxy_url(port: u16, original: &str) -> String {
    let encoded: String = form_urlencoded::byte_serialize(original.as_bytes()).collect();
    format!("http://127.0.0.1:{port}{QUERY_ROUTE_PREFIX}{}?{QUERY_PARAMETER}={encoded}", extension_of(original))
}

pub fn target_url(path: &str, query: Option<&str>) -> Option<String> {
    if path.starts_with(QUERY_ROUTE_PREFIX) {
        return form_urlencoded::parse(query?.as_bytes())
            .find(|(name, _)| name == QUERY_PARAMETER)
            .map(|(_, value)| value.into_owned())
            .filter(|value| is_http(value));
    }
    let embedded = ["/https://", "/http://"].iter().filter_map(|marker| path.rfind(marker)).max()?;
    let restored = restore_rewritten(&path[embedded + 1..]);
    Some(match query {
        Some(query) => format!("{restored}?{query}"),
        None => restored,
    })
}

pub fn parse_range(value: &str) -> Option<ByteRange> {
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (first, last) = spec.split_once('-')?;
    if first.is_empty() {
        return last.parse().ok().map(ByteRange::Suffix);
    }
    let start = first.parse().ok()?;
    let end = if last.is_empty() { None } else { Some(last.parse().ok()?) };
    Some(ByteRange::From { start, end })
}

pub fn resolve_range(range: Option<ByteRange>, total: u64) -> RangeOutcome {
    match range {
        None => RangeOutcome::Full,
        Some(ByteRange::Suffix(0)) => RangeOutcome::Unsatisfiable,
        Some(ByteRange::Suffix(length)) if total > 0 => {
            RangeOutcome::Partial { start: total.saturating_sub(length), end: total - 1 }
        }
        Some(ByteRange::From { start, end }) if start < total && end.is_none_or(|end| end >= start) => {
            RangeOutcome::Partial { start, end: end.map_or(total - 1, |end| end.min(total - 1)) }
        }
        Some(_) => RangeOutcome::Unsatisfiable,
    }
}

pub fn absolutize_playlist(text: &str, base: &str) -> String {
    text.split('\n')
        .map(|line| {
            let content = line.trim_end_matches('\r');
            if content.starts_with('#') {
                let tail = &line[content.len()..];
                return format!("{}{tail}", absolutize_tag_uris(content, base));
            }
            if content.trim().is_empty() || is_http(content) {
                return line.to_string();
            }
            let tail = &line[content.len()..];
            format!("{}{tail}", resolve_url(base, content.trim()))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn absolutize_tag_uris(tag: &str, base: &str) -> String {
    let mut rest = tag;
    let mut out = String::with_capacity(tag.len());
    while let Some(position) = rest.find(TAG_URI_OPEN) {
        let value_start = position + TAG_URI_OPEN.len();
        out.push_str(&rest[..value_start]);
        rest = &rest[value_start..];
        let Some(close) = rest.find('"') else { break };
        let value = &rest[..close];
        if value.is_empty() || Url::parse(value).is_ok() {
            out.push_str(value);
        } else {
            out.push_str(&resolve_url(base, value));
        }
        rest = &rest[close..];
    }
    out.push_str(rest);
    out
}

pub fn block_key(key: &str, block: u64) -> String {
    format!("{key}#{block}")
}

pub fn playlist_references(text: &str) -> Vec<String> {
    let mut references = Vec::new();
    for line in text.split('\n').map(|line| line.trim_end_matches('\r')) {
        if !line.starts_with('#') {
            if is_http(line) {
                references.push(line.to_string());
            }
            continue;
        }
        let mut rest = line;
        while let Some((_, after)) = rest.split_once(TAG_URI_OPEN) {
            let Some((value, tail)) = after.split_once('"') else { break };
            if is_http(value) {
                references.push(value.to_string());
            }
            rest = tail;
        }
    }
    references
}

pub fn content_type_for(url: &str, origin: Option<&str>) -> String {
    let known = match extension_of(url).as_str() {
        "mp4" | "m4v" => Some("video/mp4"),
        "ts" => Some("video/mp2t"),
        "m4s" => Some("video/iso.segment"),
        "mov" => Some("video/quicktime"),
        "webm" => Some("video/webm"),
        "aac" => Some("audio/aac"),
        "mp3" => Some("audio/mpeg"),
        "m4a" => Some("audio/mp4"),
        "m3u8" => Some("application/vnd.apple.mpegurl"),
        _ => None,
    };
    origin.or(known).unwrap_or("application/octet-stream").to_string()
}

fn is_http(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn extension_of(original: &str) -> String {
    let extension = Url::parse(original)
        .ok()
        .and_then(|parsed| parsed.path_segments().and_then(|mut segments| segments.next_back().map(str::to_string)))
        .and_then(|segment| segment.rsplit_once('.').map(|(_, extension)| extension.to_lowercase()))
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= MAX_EXTENSION_LENGTH
                && extension.chars().all(|character| character.is_ascii_alphanumeric())
        });
    extension.unwrap_or_else(|| FALLBACK_EXTENSION.to_string())
}
