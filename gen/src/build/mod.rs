use std::collections::HashMap;

use medialayer::limits::LeadingChunks;
use medialayer::spec::{AudioSpec, CacheSpec, Evict, ImageSpec, MediaSpec, VideoSpec};

use crate::parser::{Call, Node, Value};

pub type Position = (usize, usize, usize);

pub struct Built {
    pub spec: MediaSpec,
    pub cache_at: Option<Position>,
    pub blocks: HashMap<String, Position>,
}

pub fn seconds(amount: u64, unit: &str) -> Option<u32> {
    let factor = match unit {
        "second" | "seconds" => 1,
        "minute" | "minutes" => 60,
        "hour" | "hours" => 3600,
        "day" | "days" => 86400,
        _ => return None,
    };
    u32::try_from(amount.checked_mul(factor)?).ok()
}

pub fn bytes(amount: u64, unit: &str) -> Option<u64> {
    match unit {
        "mib" => amount.checked_mul(1024 * 1024),
        "gib" => amount.checked_mul(1024 * 1024 * 1024),
        _ => None,
    }
}

pub fn int(node: &Node<Value>) -> Option<u32> {
    match node.item {
        Value::Int(number) => u32::try_from(number).ok(),
        _ => None,
    }
}

pub fn duration(node: &Node<Value>) -> Option<u32> {
    match &node.item {
        Value::Quantity { amount, unit } => seconds(*amount, unit),
        _ => None,
    }
}

pub fn size(node: &Node<Value>) -> Option<u64> {
    match &node.item {
        Value::Quantity { amount, unit } => bytes(*amount, unit),
        _ => None,
    }
}

pub fn percent(node: &Node<Value>) -> Option<u32> {
    match &node.item {
        Value::Quantity { amount, unit } if unit == "percent" => u32::try_from(*amount).ok(),
        _ => None,
    }
}

pub fn symbol(node: &Node<Value>) -> Option<String> {
    match &node.item {
        Value::Symbol(name) => Some(name.clone()),
        _ => None,
    }
}

pub fn range(node: &Node<Value>) -> Option<(u32, u32)> {
    match node.item {
        Value::Range { from, to } => Some((u32::try_from(from).ok()?, u32::try_from(to).ok()?)),
        _ => None,
    }
}

pub fn chunks(node: &Node<Value>) -> Option<LeadingChunks> {
    match &node.item {
        Value::Array(items) => LeadingChunks::new(&items.iter().map(int).collect::<Option<Vec<u32>>>()?).ok(),
        _ => None,
    }
}

pub fn evict(node: &Node<Value>) -> Option<Evict> {
    match &node.item {
        Value::Symbol(name) => match name.as_str() {
            "lru" => Some(Evict::Lru),
            "fifo" => Some(Evict::Fifo),
            "lfu" => Some(Evict::Lfu),
            "largest_first" => Some(Evict::LargestFirst),
            _ => None,
        },
        Value::Call { word, arg } if word == "expire_after" => Some(Evict::ExpireAfter { seconds: duration(arg)? }),
        _ => None,
    }
}

pub fn header_name(name: &str) -> String {
    let parts: Vec<String> = name
        .split('_')
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map(|first| first.to_uppercase().chain(characters).collect()).unwrap_or_default()
        })
        .collect();
    parts.join("-")
}

pub fn headers(node: &Node<Value>) -> Option<Vec<String>> {
    match &node.item {
        Value::Array(items) => items.iter().map(|item| symbol(item).map(|name| header_name(&name))).collect(),
        _ => None,
    }
}

fn command<'a>(body: &'a [Call], word: &str) -> Option<&'a Call> {
    body.iter().find(|call| call.word == word)
}

fn argument<T>(body: &[Call], word: &str, convert: fn(&Node<Value>) -> Option<T>) -> Option<T> {
    command(body, word).and_then(|call| call.args.first()).and_then(convert)
}

fn keyed<T>(body: &[Call], word: &str, key: &str, convert: fn(&Node<Value>) -> Option<T>) -> Option<T> {
    let call = command(body, word)?;
    call.options.iter().find(|pair| pair.key == key).and_then(|pair| convert(&pair.value))
}

pub fn build(files: &[Vec<Call>]) -> Built {
    let mut built = Built { spec: MediaSpec::default(), cache_at: None, blocks: HashMap::new() };
    for (file, calls) in files.iter().enumerate() {
        for call in calls {
            let body = call.block.as_deref().unwrap_or_default();
            let at = (file, call.line, call.column);
            if call.word == "cache" {
                built.cache_at = Some(at);
                built.spec.cache = cache(body);
                continue;
            }
            let name = call.args.first().and_then(symbol).unwrap_or_default();
            built.blocks.insert(name.clone(), at);
            match call.word.as_str() {
                "video" => built.spec.videos.push(video(name, body)),
                "image" => built.spec.images.push(image(name, body)),
                _ => built.spec.audios.push(AudioSpec { name, evict: argument(body, "evict", evict) }),
            }
        }
    }
    built
}

fn cache(body: &[Call]) -> CacheSpec {
    let base = CacheSpec::default();
    CacheSpec {
        budget: argument(body, "budget", size).unwrap_or(base.budget),
        min: keyed(body, "budget", "min", size).unwrap_or(base.min),
        shrink_to: argument(body, "shrink_to", percent).unwrap_or(base.shrink_to),
        evict: argument(body, "evict", evict).unwrap_or(base.evict),
        memory: argument(body, "memory", size).unwrap_or(base.memory),
        timeout: argument(body, "timeout", duration).unwrap_or(base.timeout),
        max_concurrent: argument(body, "max_concurrent", int).unwrap_or(base.max_concurrent),
        dedupe_window: keyed(body, "dedupe", "window", duration).unwrap_or(base.dedupe_window),
        dedupe_entries: keyed(body, "dedupe", "entries", int).unwrap_or(base.dedupe_entries),
        prefetch_budget_mib: argument(body, "prefetch_budget", size).map_or(base.prefetch_budget_mib, |bytes| (bytes / (1024 * 1024)) as u32),
        namespace_headers: keyed(body, "namespace", "headers", headers).unwrap_or(base.namespace_headers),
    }
}

fn video(name: String, body: &[Call]) -> VideoSpec {
    let mut video = VideoSpec::new(&name);
    video.cover_field = argument(body, "cover", symbol);
    video.prefetch.from = keyed(body, "prefetch", "from", int).unwrap_or(video.prefetch.from);
    video.prefetch.to = keyed(body, "prefetch", "to", int).unwrap_or(video.prefetch.to);
    video.prefetch.chunks = keyed(body, "prefetch", "chunks", chunks).unwrap_or(video.prefetch.chunks);
    video.pool.slots = keyed(body, "pool", "slots", int).unwrap_or(video.pool.slots);
    video.pool.reach = keyed(body, "pool", "reach", int).unwrap_or(video.pool.reach);
    video.pool.hold_off = keyed(body, "pool", "hold_off", duration).unwrap_or(video.pool.hold_off);
    video.pool.trim_delay = keyed(body, "pool", "trim_delay", duration).unwrap_or(video.pool.trim_delay);
    video.pool.low_ram_slots = keyed(body, "pool", "low_ram_slots", int);
    video.pool.low_ram_reach = keyed(body, "pool", "low_ram_reach", int);
    video.forward_headers = argument(body, "forward_headers", headers).unwrap_or(video.forward_headers);
    video.evict = argument(body, "evict", evict);
    video
}

fn image(name: String, body: &[Call]) -> ImageSpec {
    let dimensions = command(body, "size").and_then(|call| Some((int(call.args.first()?)?, int(call.args.get(1)?)?)));
    ImageSpec {
        name,
        blurhash_field: argument(body, "blurhash", symbol),
        size: dimensions,
        prefetch_window: keyed(body, "prefetch", "window", range),
        prefetch_behind: keyed(body, "prefetch", "behind", int),
        evict: argument(body, "evict", evict),
    }
}
