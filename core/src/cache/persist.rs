use super::entry::{Entry, Kind};
use super::naming::{hex_decode, hex_encode};

pub fn encode_line(key: &str, entry: &Entry) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\n",
        hex_encode(key),
        entry.size,
        entry.kind.code(),
        entry.created,
        entry.last_access,
        entry.hits
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
    if fields.next().is_some() {
        return None;
    }
    Some((key, Entry { size, kind, created, last_access, hits }))
}
