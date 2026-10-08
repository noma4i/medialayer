use std::fs;
use std::path::PathBuf;

use crate::{kotlin, parse_files};

#[test]
fn yupi_fixture_matches_the_golden_kotlin_file() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let spec = parse_files(&[root.join("fixtures/yupi.mdl")]).unwrap();
    assert_eq!(kotlin(&spec, "app.media"), fs::read_to_string(root.join("fixtures/golden/MediaSources.kt")).unwrap());
}
