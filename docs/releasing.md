# Releasing

The owner chooses the version and authorizes publication. The release script never chooses a version.

## Steps

1. Replace `## Unreleased` in `CHANGELOG.md` with `## X.Y.Z - YYYY-MM-DD`. Keep non-empty sections in `Added`, `Changed`, `Fixed`, `Removed` order. Use one sentence of at most 15 words per entry.
2. Update `core/Cargo.toml`, `gen/Cargo.toml`, their `Cargo.lock` entries, and the Swift and Kotlin `version()` assertions. Commit these changes and the changelog as `Release X.Y.Z` on `master`.
3. On brick, put Gradle 9 in `PATH` or set `GRADLE` to its executable. Run `make release-android VERSION=X.Y.Z OUT_DIR=/path/to/android-result`. This builds all four Android ABIs, stages all three Maven modules, and checks the AAR files.
4. Copy the result directory to the Mac. Run `bash scripts/release.sh X.Y.Z --prepare --android /path/to/android-result` to build the release stage.
5. Run `bash scripts/check_medialayer.sh --report-lane mac --report /path/to/mac.json` on the Mac. Run `bash scripts/check_medialayer.sh --report-lane brick --report /path/to/brick.json` on brick. Copy the brick report to the Mac. A report stores the input hash of each step (the hash of `git ls-tree -r HEAD` for the step input paths in `scripts/check_medialayer.sh`). A rerun with the same `--report` file skips each green step whose input hash is unchanged and prints `reused <step>`, so a Swift-only edit never reruns a Kotlin step. The merge and `make release` accept a step only when its hash equals the hash of its inputs now.
6. Run `python3 scripts/release_evidence.py merge /path/to/mac.json /path/to/brick.json /path/to/android-result target/release-X.Y.Z /path/to/checks.json`. The merge checks public symbols, owners and arity. JVM erasure omits generic and nullable types. Run `make release VERSION=X.Y.Z ANDROID_DIR=/path/to/android-result CHECK_REPORT=/path/to/checks.json`.
7. After the owner authorizes publication, run `make release-publish VERSION=X.Y.Z ANDROID_DIR=/path/to/android-result CHECK_REPORT=/path/to/checks.json`. This commits the package manifest and generated Swift bridge as `Package release X.Y.Z`, creates the tag, pushes `master` and the tag, pushes Maven files to `maven`, then creates the GitHub Release. The tag is lightweight and points at the `Package release X.Y.Z` commit.
8. Check the GitHub Release title, changelog text, and four attachment names. The script downloads the attachments and compares their hashes with the local stage.

The release page title is the version. Its text is only that version's `CHANGELOG.md` section. Attachments are only the compiled files listed in [Getting started](getting-started.md#use-a-release). Checksums remain in the ignored stage directory.

The release script requires a clean `master` tree, the Android result directory, and exactly one `Release X.Y.Z` commit since the previous tag. It checks both Maven coordinates, module metadata, AAR contents, the Android archive, documentation, iOS archive contents, architectures, and generated manifest before creating the package commit or tag.

Android coordinates are `com.noma4i:medialayer:X.Y.Z`, `com.noma4i:medialayer-ui:X.Y.Z` and `com.noma4i:medialayer-testing:X.Y.Z`. The Maven branch contains `com/noma4i/medialayer/X.Y.Z/`, `com/noma4i/medialayer-ui/X.Y.Z/` and `com/noma4i/medialayer-testing/X.Y.Z/` at its root. Publication uses local Maven staging and does not need credentials.

The root `Package.swift` uses the release URL and checksum for SwiftPM. Keep `MediaLayer/Package.swift` and `scripts/build_ios.sh` for local development.

Rule INV-CAPTURE-80: no tag without the green recorded-video guard on both platforms: Swift `TakeTransformTests` and Kotlin `CaptureUprightTest` pass in the full run.
