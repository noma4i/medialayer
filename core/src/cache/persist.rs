use super::entry::{Check, Entry, Kind};
use super::naming::{hex_decode, hex_decode_bytes};
use crate::analysis::ToneData;
use crate::identity::hex_into;
use crate::proxy::Meta;
use std::fmt::Write;

pub fn encode_line(key: &str, entry: &Entry, text: &mut String) {
    hex_into(key.as_bytes(), text);
    let _ = write!(text, "\t{}\t{}\t{}\t{}\t{}\t", entry.size, entry.kind.code(), entry.created, entry.last_access, entry.hits);
    if let Some(content_type) = &entry.meta.content_type {
        hex_into(content_type.as_bytes(), text);
    }
    text.push('\t');
    if let Some(length) = entry.meta.total_length {
        let _ = write!(text, "{length}");
    }
    text.push('\t');
    if let Check::Sum(sum) = entry.check {
        let _ = write!(text, "{sum:016x}");
    }
    text.push('\t');
    if let Some(tone) = &entry.tone {
        hex_into(&tone.encode(), text);
    }
    text.push('\n');
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
    let check = match fields.next() {
        None | Some("") => Check::Legacy,
        Some(sum) => Check::Sum(u64::from_str_radix(sum, 16).ok()?),
    };
    let tone = fields.next().and_then(|field| hex_decode_bytes(field).and_then(|bytes| ToneData::decode(&bytes)));
    if fields.next().is_some() {
        return None;
    }
    Some((key, Entry { size, check, kind, created, last_access, hits, meta: Meta { content_type, total_length }, tone }))
}

fn optional<T>(field: &str, parse: impl FnOnce(&str) -> Option<T>) -> Option<Option<T>> {
    if field.is_empty() {
        return Some(None);
    }
    parse(field).map(Some)
}
