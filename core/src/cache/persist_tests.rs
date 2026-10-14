use super::naming::{hex_decode, hex_encode};
use super::persist::{decode_line, encode_line};
use super::*;
use crate::cache::entry::Entry;
use crate::proxy::Meta;

fn sample(meta: Meta) -> Entry {
    Entry { check: super::entry::Check::Legacy, size: 42, kind: Kind::Stream, created: 7, last_access: 9, hits: 3, meta, tone: None }
}

#[test]
fn inv_cache_14_line_round_trips_with_and_without_meta() {
    let metas = [
        Meta::default(),
        Meta { content_type: Some("video/mp4; codecs=\"avc1\"".to_string()), total_length: None },
        Meta { content_type: None, total_length: Some(1_048_577) },
        Meta { content_type: Some("image/webp".to_string()), total_length: Some(0) },
    ];
    for meta in metas {
        let line = encode_line("some key/é", &sample(meta.clone()));
        assert_eq!(decode_line(line.trim_end_matches('\n')), Some(("some key/é".to_string(), sample(meta))));
    }
}

#[test]
fn inv_cache_14_malformed_lines_are_rejected() {
    for line in [
        "zz\t1\timage\t1\t1\t1\t\t",
        "6b\t1\tvideo\t1\t1\t1\t\t",
        "6b\t1\timage\t1\t1\t1\t",
        "6b\t1\timage\t1\t1\t1\t\t\textra",
        "6b\t1\timage\t1\t1\t1\tzz\t",
        "6b\t1\timage\t1\t1\t1\t\tmany",
    ] {
        assert_eq!(decode_line(line), None, "{line:?}");
    }
}

#[test]
fn inv_cache_14_hex_names_round_trip() {
    assert_eq!(hex_decode(&hex_encode("a/b c")), Some("a/b c".to_string()));
    assert_eq!(hex_decode("abc"), None);
}

fn toned(level: u8) -> Entry {
    let pixels = [[level, level, level, 255]; 4].concat();
    Entry { tone: Some(crate::analysis::analyze(2, 2, &pixels, crate::analysis::ToneSource::Pixels).unwrap()), ..sample(Meta::default()) }
}

#[test]
fn inv_tone_08_a_line_carries_the_tone_and_a_damaged_tone_field_keeps_the_entry_without_it() {
    let entry = toned(33);
    let line = encode_line("tone key", &entry);

    assert_eq!(decode_line(line.trim_end_matches('\n')), Some(("tone key".to_string(), entry.clone())));
    let damaged = line.trim_end_matches('\n').rsplit_once('\t').map(|(head, _)| format!("{head}\tzz")).unwrap();
    assert_eq!(decode_line(&damaged), Some(("tone key".to_string(), Entry { tone: None, ..entry })));
}
