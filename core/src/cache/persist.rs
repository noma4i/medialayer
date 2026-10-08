use super::entry::{Entry, Kind};
use super::naming::{hex_decode, hex_encode};
use crate::proxy::Meta;

pub fn encode_line(key: &str, entry: &Entry) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        hex_encode(key),
        entry.size,
        entry.kind.code(),
        entry.created,
        entry.last_access,
        entry.hits,
        entry.meta.content_type.as_deref().map(hex_encode).unwrap_or_default(),
        entry.meta.total_length.map(|length| length.to_string()).unwrap_or_default(),
    )
}

pub fn decode_line(line: &str) -> Option<(String, Entry)> {
    let mut fields = line.split('\t');
    let key = hex_decode(fields.next()?)?;
    let size = fields.next()?.parse().ok()?;
    let kind = Kind::from_code(fields.next()?)?;
    let created = fields.next()?.parse().ok()?;
    let last_access = fields.next()?.parse().ok()?;
    let hits = fields.next()?.parse().ok()?;
    let content_type = optional(fields.next()?, hex_decode)?;
    let total_length = optional(fields.next()?, |text| text.parse().ok())?;
    if fields.next().is_some() {
        return None;
    }
    Some((key, Entry { size, kind, created, last_access, hits, meta: Meta { content_type, total_length } }))
}

fn optional<T>(field: &str, parse: impl FnOnce(&str) -> Option<T>) -> Option<Option<T>> {
    if field.is_empty() {
        return Some(None);
    }
    parse(field).map(Some)
}
