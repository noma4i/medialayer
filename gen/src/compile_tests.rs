use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use crate::{parse_files, MediaSpec};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn spec() -> MediaSpec {
    parse_files(&[fixtures().join("yupi.mdl"), fixtures().join("compile.mdl")]).unwrap()
}

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must explicitly name the compiler or library configuration"))
}

fn compile(command: &mut Command) -> Output {
    command.output().unwrap_or_else(|error| panic!("cannot run {command:?}: {error}"))
}

fn missing_compiler(test: &str, variable: &str) {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(variable, directory.path().join("missing-compiler"))
        .output()
        .unwrap();
    assert!(!output.status.success(), "missing compiler passed:\n{}", String::from_utf8_lossy(&output.stdout));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot run"), "{}", String::from_utf8_lossy(&output.stderr));
}

#[cfg(feature = "compile-swift")]
#[test]
fn inv_gen_03_generated_swift_compiles_with_the_library() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("MediaSources.swift");
    fs::write(&file, crate::swift(&spec())).unwrap();
    let args: Vec<String> = serde_json::from_str(&required("SWIFT_CHECK_ARGS")).expect("SWIFT_CHECK_ARGS must be a JSON array");
    let output = compile(Command::new(required("SWIFTC"))
        .args(args)
        .arg("-typecheck")
        .arg(&file)
        .arg(fixtures().join("Consumer.swift")));
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[cfg(feature = "compile-swift")]
#[test]
fn inv_gen_03_missing_swift_compiler_fails_the_selected_check() {
    missing_compiler("compile_tests::inv_gen_03_generated_swift_compiles_with_the_library", "SWIFTC");
}

#[cfg(feature = "compile-kotlin")]
#[test]
fn inv_gen_03_generated_kotlin_is_placed_in_the_gradle_test_sources() {
    let directory = fixtures().join("../../kotlin/src/test/kotlin/app/media");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("MediaSources.kt"), crate::kotlin(&spec(), "app.media")).unwrap();
    fs::copy(fixtures().join("Consumer.kt"), directory.join("Consumer.kt")).unwrap();
    assert!(directory.join("Consumer.kt").is_file() && directory.join("MediaSources.kt").is_file());
}
