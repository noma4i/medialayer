use std::fs;
use std::path::PathBuf;
use std::process::Command;

use medialayer::spec::{AudioSpec, CacheSpec, Evict, ImageSpec, MediaSpec, PoolSpec, PrefetchSpec, VideoSpec};

use super::*;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn ruby_blocks(path: &str) -> Vec<String> {
    let text = fs::read_to_string(root().join("..").join(path)).unwrap();
    text.split("```ruby\n").skip(1).map(|chunk| chunk.split("```").next().unwrap().to_string()).collect()
}

fn media_files_spec() -> MediaSpec {
    let mut video = VideoSpec::new("moment_video");
    video.cover_field = Some("coverUrl".into());
    video.prefetch = PrefetchSpec { from: 1, to: 5, chunks: medialayer::limits::LeadingChunks::new(&[3, 2, 1]).unwrap() };
    video.pool = PoolSpec { slots: 3, reach: 1, hold_off: 1, trim_delay: 30, low_ram_slots: Some(2), low_ram_reach: Some(0) };
    video.forward_headers = vec!["Authorization".into(), "Cookie".into(), "User-Agent".into()];
    video.evict = Some(Evict::Lfu);
    MediaSpec {
        cache: CacheSpec {
            memory: 256 * 1024 * 1024,
            namespace_headers: vec!["Authorization".into()],
            ..CacheSpec::default()
        },
        videos: vec![video],
        images: vec![ImageSpec {
            name: "avatar".into(),
            blurhash_field: Some("blurHash".into()),
            size: Some((288, 288)),
            prefetch_window: Some((1, 3)),
            prefetch_behind: Some(2),
            evict: None,
        }],
        audios: vec![AudioSpec { name: "voice_note".into(), evict: Some(Evict::Fifo) }],
    }
}

#[test]
fn the_media_files_reference_example_builds_the_documented_spec() {
    let blocks = ruby_blocks("docs/media-files.md");
    assert_eq!(blocks.len(), 1);
    assert_eq!(parse_spec(&blocks[0]).unwrap(), media_files_spec());
}

#[test]
fn every_ruby_example_in_the_docs_parses() {
    for path in ["README.md", "docs/memory.md", "docs/media-files.md"] {
        for block in ruby_blocks(path) {
            assert!(parse_spec(&block).is_ok(), "{path}: {block}");
        }
    }
}

#[test]
fn expire_after_and_every_duration_unit_reach_the_spec() {
    let spec = parse_spec("cache do\n  evict expire_after 7.days\n  timeout 2.minutes\nend\nvideo :v do\n  pool hold_off: 1.hour\nend\n").unwrap();
    assert_eq!((spec.cache.evict, spec.cache.timeout, spec.videos[0].pool.hold_off), (Evict::ExpireAfter { seconds: 604800 }, 120, 3600));
}

#[test]
fn the_low_memory_pool_options_reach_the_spec_and_a_bad_value_names_its_place() {
    let spec = parse_spec("video :v do\n  pool low_ram_slots: 4, low_ram_reach: 1\nend\nvideo :w do\nend\n").unwrap();
    assert_eq!((spec.videos[0].pool.low_ram_slots, spec.videos[0].pool.low_ram_reach), (Some(4), Some(1)));
    assert_eq!((spec.videos[1].pool.low_ram_slots, spec.videos[1].pool.low_ram_reach), (None, None));

    let errors = parse_spec("video :v do\n  pool low_ram_slots: 1.second\nend\n").unwrap_err();
    assert_eq!(errors[0].message, "`low_ram_slots:` must be a whole number");
}

#[test]
fn several_files_merge_and_errors_name_their_file() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("a.mdl");
    let second = directory.path().join("b.mdl");
    fs::write(&first, "video :a do\nend\n").unwrap();
    fs::write(&second, "audio :b do\nend\n").unwrap();
    let spec = parse_files(&[&first, &second]).unwrap();
    assert_eq!((spec.videos.len(), spec.audios.len()), (1, 1));
    fs::write(&second, "audio :a do\nend\n").unwrap();
    let errors = parse_files(&[&first, &second]).unwrap_err();
    assert_eq!(errors[0].to_string(), format!("{}:1:7: `a` is declared twice", second.display()));
    assert_eq!(parse_files(&["/nonexistent/none.mdl"]).unwrap_err()[0].file, "/nonexistent/none.mdl");
}

#[test]
fn generated_swift_is_valid_swift() {
    let spec = parse_files(&[root().join("fixtures/yupi.mdl")]).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("MediaSources.swift");
    fs::write(&file, swift(&spec)).unwrap();
    let output = Command::new("swiftc").arg("-parse").arg(&file).output();
    match output {
        Ok(output) => assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr)),
        Err(_) => eprintln!("swiftc is not installed: swift syntax check skipped"),
    }
}

#[test]
fn generated_kotlin_is_valid_kotlin() {
    let spec = parse_files(&[root().join("fixtures/yupi.mdl")]).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("MediaSources.kt");
    fs::write(&file, kotlin(&spec, "app.media")).unwrap();
    match Command::new("kotlinc").arg(&file).arg("-d").arg(directory.path()).output() {
        Ok(output) => assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr)),
        Err(_) => eprintln!("kotlinc is not installed: kotlin syntax check skipped"),
    }
}
