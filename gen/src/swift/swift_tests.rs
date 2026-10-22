use std::fs;
use std::path::PathBuf;

use crate::{parse_files, swift};

#[test]
fn feed_fixture_matches_the_golden_swift_file() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let spec = parse_files(&[root.join("fixtures/feed.mdl")]).unwrap();
    assert_eq!(swift(&spec), fs::read_to_string(root.join("fixtures/golden/MediaSources.swift")).unwrap());
}
