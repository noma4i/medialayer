use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use crate::{parse_files, MediaSpec};

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn spec() -> MediaSpec {
    parse_files(&[fixtures().join("runtime.mdl")]).unwrap()
}

fn expected_json() -> String {
    crate::lib_tests::runtime_spec().to_json()
}

fn required(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must explicitly name the compiler or library configuration"))
}

fn compile(command: &mut Command) -> Output {
    command.output().unwrap_or_else(|error| panic!("cannot run {command:?}: {error}"))
}

fn documentation_example(document: &str, language: &str) -> String {
    let path = fixtures().join("../../docs").join(document);
    let text = fs::read_to_string(path).unwrap();
    text.split_once(&format!("```{language}\n"))
        .unwrap()
        .1
        .split_once("```")
        .unwrap()
        .0
        .to_owned()
}

#[cfg(feature = "compile-swift")]
struct Placed(PathBuf);

#[cfg(feature = "compile-swift")]
impl Drop for Placed {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(feature = "compile-swift")]
#[test]
fn inv_gen_03_generated_swift_compiles_with_the_library() {
    let runner = required("SWIFT_TEST_RUNNER");
    compile(Command::new(&runner).arg("--version"));
    let placed = Placed(fixtures().join("../../MediaLayer/Tests/MediaLayerTests/Generated"));
    fs::create_dir_all(&placed.0).unwrap();
    fs::write(placed.0.join("MediaSources.swift"), crate::swift(&spec())).unwrap();
    fs::copy(fixtures().join("Consumer.swift"), placed.0.join("Consumer.swift")).unwrap();
    fs::write(placed.0.join("Documentation.swift"), format!("import MediaLayer\n\nfunc documentationLimits(media: MediaLayer, url: String) async throws {{\n{}\n}}\n\nfunc documentationPool(media: MediaLayer, introItem: MediaItem) throws {{\n{}\n}}\n", documentation_example("limits-and-eviction.md", "swift"), documentation_example("player-pool.md", "swift"))).unwrap();
    fs::write(placed.0.join("ExpectedSpec.swift"), format!("enum ExpectedSpecJSON {{\n    static let value = #\"\"\"\n{}\n\"\"\"#\n}}\n", expected_json())).unwrap();
    let args: Vec<String> = serde_json::from_str(&required("SWIFT_TEST_ARGS")).expect("SWIFT_TEST_ARGS must be a JSON array");
    let output = compile(Command::new(&runner).args(args));
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "{text}");
    println!("{text}");
    for test in [
        "inv_strategy_15_generated_sources_turn_into_media_items_with_the_dsl_values",
        "inv_gen_03_the_embedded_spec_json_is_the_dsl_data",
        "inv_gen_03_the_generated_spec_reaches_the_core_with_every_dsl_option",
    ] {
        assert!(text.contains(test), "the generated test {test} did not run:\n{text}");
    }
}

#[cfg(feature = "compile-kotlin")]
#[test]
fn inv_gen_03_generated_kotlin_is_placed_in_the_gradle_test_sources() {
    let directory = fixtures().join("../../kotlin/src/test/kotlin/app/media");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("MediaSources.kt"), crate::kotlin(&spec(), "app.media")).unwrap();
    fs::copy(fixtures().join("Consumer.kt"), directory.join("Consumer.kt")).unwrap();
    fs::write(directory.join("Documentation.kt"), format!("package app.media\n\nimport com.noma4i.medialayer.*\n\nsuspend fun documentationLimits(media: MediaLayer, url: String) {{\n{}\n}}\n\nfun documentationPool(media: MediaLayer, introItem: MediaItem) {{\n{}\n}}\n", documentation_example("limits-and-eviction.md", "kotlin"), documentation_example("player-pool.md", "kotlin"))).unwrap();
    fs::write(directory.join("ExpectedSpec.kt"), format!("package app.media\n\nobject ExpectedSpecJson {{\n    const val VALUE: String = \"\"\"{}\"\"\"\n}}\n", expected_json())).unwrap();
    assert!(directory.join("Consumer.kt").is_file() && directory.join("MediaSources.kt").is_file());
}
