use super::naming::{hex_decode_bytes, hex_encode};
use super::persist::{decode_line, encode_line};
use super::*;
use crate::cache::entry::Entry;
use crate::proxy::Meta;

#[tokio::test]
async fn inv_cache_23_incremental_save_writes_only_changed_records() {
    let f = super::test_support::fixture_with_slow_index(crate::limits::Limits::default()).await;
    for index in 0..100 {
        f.cache.put(&format!("https://cache.test/{index}"), Kind::Image, b"x").await.unwrap();
    }
    f.cache.flush().await.unwrap();
    let before = f.cache.inner.index_bytes.load(std::sync::atomic::Ordering::Relaxed);
    f.cache.put("https://cache.test/0", Kind::Image, b"changed").await.unwrap();
    let name = super::naming::file_name("https://cache.test/0");
    let entry = f.cache.inner.state().slot(&name).unwrap().entry.clone();
    let mut record = String::new();
    encode_line("https://cache.test/0", &entry, &mut record);
    f.cache.flush().await.unwrap();
    let written = f.cache.inner.index_bytes.load(std::sync::atomic::Ordering::Relaxed) - before;
    eprintln!("changed_record_bytes={} index_written={written}", record.len());
    assert!(written <= record.len() as u64, "unchanged records were rewritten");
}

#[tokio::test]
async fn inv_cache_24_unknown_index_version_discards_entries() {
    let f = super::test_support::fixture(crate::limits::Limits::default(), &[]).await;
    f.cache.put("clip", Kind::Stream, b"data").await.unwrap();
    f.cache.flush().await.unwrap();
    let path = f.dir.path().join(super::naming::INDEX_FILE);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("unknown-version\n{text}")).unwrap();
    let reopened = Cache::open(f.dir.path(), crate::limits::Limits::default());
    reopened.ready().await;
    assert!(!reopened.contains("clip").await);
}

fn sample(meta: Meta) -> Entry {
    Entry { check: super::entry::Check(vec![0; 1].into()), size: 42, kind: Kind::Stream, created: 7, last_access: 9, hits: 3, meta, tone: None, image_size: None }
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
        let mut line = String::new();
        encode_line("some key/é", &sample(meta.clone()), &mut line);
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
    assert_eq!(hex_decode_bytes(&hex_encode("a/b c")), Some(b"a/b c".to_vec()));
    assert_eq!(hex_decode_bytes("abc"), None);
}

fn toned(level: u8) -> Entry {
    let pixels = [[level, level, level, 255]; 4].concat();
    Entry { tone: Some(crate::analysis::analyze(2, 2, &pixels, crate::analysis::ToneSource::Pixels).unwrap()), ..sample(Meta::default()) }
}

#[test]
fn inv_tone_08_a_line_carries_the_tone_and_a_damaged_tone_field_keeps_the_entry_without_it() {
    let entry = toned(33);
    let mut line = String::new();
    encode_line("tone key", &entry, &mut line);

    assert_eq!(decode_line(line.trim_end_matches('\n')), Some(("tone key".to_string(), entry.clone())));
    let mut fields = line.trim_end_matches('\n').rsplitn(3, '\t');
    let (size, _, head) = (fields.next().unwrap(), fields.next().unwrap(), fields.next().unwrap());
    let damaged = format!("{head}\tzz\t{size}");
    assert_eq!(decode_line(&damaged), Some(("tone key".to_string(), Entry { tone: None, ..entry })));
}

#[tokio::test]
async fn inv_cache_07_failed_index_save_preserves_dirty_state() {
    use std::sync::atomic::Ordering;
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::open(dir.path(), crate::limits::Limits::default());
    cache.ready().await;
    std::fs::create_dir(dir.path().join(super::naming::INDEX_TMP_FILE)).unwrap();
    cache.inner.dirty.store(false, Ordering::SeqCst);
    assert!(cache.flush().await.is_err());
    assert!(cache.inner.dirty.load(Ordering::SeqCst), "failed flush discarded the retry requirement");
    std::fs::remove_dir(dir.path().join(super::naming::INDEX_TMP_FILE)).unwrap();
    cache.flush().await.unwrap();
    super::test_support::wait_until(|| dir.path().join(super::naming::INDEX_FILE).is_file()).await;
    assert!(!cache.inner.dirty.load(Ordering::SeqCst));
}

#[tokio::test]
async fn inv_strategy_12_clear_and_purge_remove_keep_history() {
    for purge in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path(), crate::limits::Limits::default());
        cache.ready().await;
        cache.put("clip#0", Kind::Stream, b"data").await.unwrap();
        cache.keep("clip");
        if purge {
            cache.purge_prefix("clip").await;
        } else {
            cache.clear(None).await;
        }
        assert!(!cache.inner.state().is_kept("clip#0"), "deleted source retained keep history");
    }
}
