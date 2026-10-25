use super::entry::{Check, Entry, Kind};
use super::naming::hex_decode_bytes;
use crate::analysis::ToneData;
use crate::identity::hex_into;
use crate::image_size::ImageSize;
use crate::proxy::Meta;
use std::fmt::Write;

const SUM_DIGITS: usize = 16;

pub const HEADER: &str = "medialayer-cache-v4\n";

pub fn encode_line(key: &str, entry: &Entry, text: &mut String) {
    encode_string(key, text);
    let _ = write!(text, "\t{}\t{}\t{}\t{}\t{}\t", entry.size, entry.kind.code(), entry.created, entry.last_access, entry.hits);
    if let Some(content_type) = &entry.meta.content_type {
        encode_string(content_type, text);
    }
    text.push('\t');
    if let Some(length) = entry.meta.total_length {
        let _ = write!(text, "{length}");
    }
    text.push('\t');
    for sum in entry.check.0.iter() {
        let _ = write!(text, "{sum:016x}");
    }
    text.push('\t');
    if let Some(tone) = &entry.tone {
        hex_into(&tone.encode(), text);
    }
    text.push('\t');
    if let Some(size) = &entry.image_size {
        text.push_str(&size.encode());
    }
    text.push('\n');
}

pub fn decode_line(line: &str) -> Option<(String, Entry)> {
    let mut fields = line.split('\t');
    let key = serde_json::from_str(fields.next()?).ok()?;
    let size = fields.next()?.parse().ok()?;
    let kind = Kind::from_code(fields.next()?)?;
    let created = fields.next()?.parse().ok()?;
    let last_access = fields.next()?.parse().ok()?;
    let hits = fields.next()?.parse().ok()?;
    let content_type = optional(fields.next()?, |text| serde_json::from_str(text).ok())?;
    let total_length = optional(fields.next()?, |text| text.parse().ok())?;
    let check = decode_sums(fields.next()?, size)?;
    let tone = fields.next().and_then(|field| hex_decode_bytes(field).and_then(|bytes| ToneData::decode(&bytes)));
    let image_size = fields.next().and_then(ImageSize::decode);
    if fields.next().is_some() {
        return None;
    }
    Some((key, Entry { size, check, kind, created, last_access, hits, meta: Meta { content_type, total_length }, tone, image_size }))
}

fn decode_sums(field: &str, size: u64) -> Option<Check> {
    if !field.len().is_multiple_of(SUM_DIGITS) || (field.len() / SUM_DIGITS) as u64 != Check::segments_for(size) {
        return None;
    }
    let sums: Option<Vec<u64>> = field.as_bytes().chunks(SUM_DIGITS).map(|digits| u64::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()).collect();
    Some(Check(sums?.into()))
}

fn optional<T>(field: &str, parse: impl FnOnce(&str) -> Option<T>) -> Option<Option<T>> {
    if field.is_empty() {
        return Some(None);
    }
    parse(field).map(Some)
}

fn encode_string(value: &str, text: &mut String) {
    text.push('"');
    for character in value.chars() {
        match character {
            '"' => text.push_str("\\\""),
            '\\' => text.push_str("\\\\"),
            character if character <= '\u{1f}' => {
                let _ = write!(text, "\\u{:04x}", character as u32);
            }
            character => text.push(character),
        }
    }
    text.push('"');
}
