use super::naming::{hex_decode, hex_encode};
use super::persist::{decode_line, encode_line};
use super::*;
use crate::cache::entry::Entry;

fn sample() -> Entry {
    Entry { size: 42, kind: Kind::Stream, created: 7, last_access: 9, hits: 3 }
}

#[test]
fn line_round_trips() {
    let line = encode_line("some key/é", &sample());
    assert_eq!(decode_line(line.trim_end()), Some(("some key/é".to_string(), sample())));
}

#[test]
fn malformed_lines_are_rejected() {
    assert_eq!(decode_line("zz\t1\timage\t1\t1\t1"), None);
    assert_eq!(decode_line("6b\t1\tvideo\t1\t1\t1"), None);
    assert_eq!(decode_line("6b\t1\timage\t1\t1"), None);
    assert_eq!(decode_line("6b\t1\timage\t1\t1\t1\textra"), None);
}

#[test]
fn hex_names_round_trip() {
    assert_eq!(hex_decode(&hex_encode("a/b c")), Some("a/b c".to_string()));
    assert_eq!(hex_decode("abc"), None);
}
