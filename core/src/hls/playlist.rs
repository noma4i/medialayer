use super::Chunk;
use bytes::Bytes;
use url::Url;

pub struct Playlist {
    pub body: Bytes,
    pub references: Vec<String>,
    pub variant: Option<String>,
    pub chunks: Vec<Chunk>,
    pub durations: Vec<u64>,
    pub target_ms: u64,
    pub is_master: bool,
    pub is_static: bool,
    pub is_vod: bool,
}

impl Playlist {
    pub fn parse(text: &str, base: &str) -> Self {
        #[cfg(test)]
        { super::work::url(); super::work::pass(); }
        let base = Url::parse(base).ok();
        let mut result = Self { body: Bytes::new(), references: Vec::new(), variant: None, chunks: Vec::new(), durations: Vec::new(), target_ms: 0, is_master: false, is_static: false, is_vod: false };
        let mut body = String::with_capacity(text.len());
        let mut first_line = true;
        let mut streams = 0;
        let mut dropping = false;
        let mut variant_next = false;
        let mut init = None;
        let mut duration = None;
        let mut timing_valid = true;
        let mut has_segments = false;
        for line in text.split('\n') {
            let trimmed = line.trim();
            let is_uri = !trimmed.is_empty() && !trimmed.starts_with('#');
            let mut resolved = String::new();
            if is_uri {
                let url = resolve(&base, trimmed);
                if is_http(&url) { result.references.push(url.clone()); }
                if variant_next && result.variant.is_none() { result.variant = Some(url.clone()); }
                if !dropping && !variant_next {
                    result.chunks.push(Chunk { segment: url.clone(), init: init.clone() });
                    match duration.take() {
                        Some(ms) => result.durations.push(ms),
                        None => timing_valid = false,
                    }
                }
                if is_http(&url) { resolved.push_str("./"); }
                resolved.push_str(&url);
                if line.ends_with('\r') { resolved.push('\r'); }
            } else {
                let mut rest = line;
                while let Some((prefix, after)) = rest.split_once("URI=\"") {
                    resolved.push_str(prefix);
                    resolved.push_str("URI=\"");
                    let Some((uri, tail)) = after.split_once('"') else { rest = after; break };
                    let url = if uri.is_empty() { String::new() } else { resolve(&base, uri) };
                    if trimmed.starts_with("#EXT-X-MAP") && !dropping { init = Some(url.clone()); }
                    if is_http(&url) {
                        result.references.push(url.clone());
                        resolved.push_str("./");
                    }
                    resolved.push_str(&url);
                    resolved.push('"');
                    rest = tail;
                }
                resolved.push_str(rest);
            }
            if trimmed.starts_with("#EXT-X-STREAM-INF") {
                streams += 1;
                result.is_master = true;
                dropping = streams > 1;
                variant_next = true;
            } else {
                variant_next = false;
                if let Some(value) = trimmed.strip_prefix("#EXT-X-TARGETDURATION:") {
                    match value.parse::<u64>().ok().and_then(|seconds| seconds.checked_mul(1000)) {
                        Some(ms) => result.target_ms = ms,
                        None => timing_valid = false,
                    }
                }
                if let Some(value) = trimmed.strip_prefix("#EXTINF:") {
                    has_segments = true;
                    duration = value.split(',').next().and_then(|value| value.trim().parse::<f64>().ok()).map(|seconds| (seconds * 1000.0).round() as u64);
                }
                if trimmed == "#EXT-X-ENDLIST" { result.is_vod = true; }
                if dropping && is_uri { dropping = false; continue; }
            }
            if !dropping {
                if !first_line { body.push('\n'); }
                first_line = false;
                body.push_str(&resolved);
            }
        }
        result.is_static = result.is_vod || (result.is_master && !has_segments);
        result.is_vod &= timing_valid && !result.chunks.is_empty() && result.chunks.len() == result.durations.len();
        if result.target_ms == 0 {
            result.target_ms = result.durations.iter().copied().max().unwrap_or(0).div_ceil(1000).saturating_mul(1000);
        }
        body.shrink_to_fit();
        result.body = Bytes::from(body);
        result
    }

    pub fn memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.body.len()
            + self.references.capacity() * std::mem::size_of::<String>() + self.references.iter().map(String::capacity).sum::<usize>()
            + self.variant.as_ref().map_or(0, String::capacity)
            + self.chunks.capacity() * std::mem::size_of::<Chunk>() + self.chunks.iter().map(|chunk| chunk.segment.capacity() + chunk.init.as_ref().map_or(0, String::capacity)).sum::<usize>()
            + self.durations.capacity() * std::mem::size_of::<u64>()
    }
}

fn resolve(base: &Option<Url>, value: &str) -> String {
    base.as_ref().and_then(|base| base.join(value).ok()).map(String::from).unwrap_or_else(|| value.to_string())
}

fn is_http(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}
