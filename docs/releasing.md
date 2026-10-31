# Releasing

The owner chooses the version and authorizes publication. The release script never chooses a version.

## Steps

1. Replace `## Unreleased` in `CHANGELOG.md` with `## X.Y.Z - YYYY-MM-DD`. Keep non-empty sections in `Added`, `Changed`, `Fixed`, `Removed` order. Use one sentence of at most 15 words per entry.
2. Update `core/Cargo.toml`, `gen/Cargo.toml`, their `Cargo.lock` entries, and the Swift and Kotlin `version()` assertions. Commit these changes and the changelog as `Release X.Y.Z` on `master`.
3. On brick, run `make release-android VERSION=X.Y.Z OUT_DIR=/path/to/android-result`. This builds all four Android ABIs, stages both Maven modules, and checks the AAR files.
4. Copy the result directory to the Mac. Run `make release VERSION=X.Y.Z ANDROID_DIR=/path/to/android-result`. This checks Android artifacts before the iOS build and prints the generated root `Package.swift` without publishing.
5. After the owner authorizes publication, run `make release-publish VERSION=X.Y.Z ANDROID_DIR=/path/to/android-result`. This commits the package manifest and generated Swift bridge as `Package release X.Y.Z`, creates the tag, pushes `master` and the tag, pushes Maven files to `maven`, pushes the four release files to `artifacts`, then creates the GitHub Release. The tag is lightweight and points at the `Package release X.Y.Z` commit.
6. Check the GitHub Release title, changelog text, and four attachment names. The script downloads the attachments and compares their hashes with the local stage.

The release page title is the version. Its text is only that version's `CHANGELOG.md` section. Attachments are only the compiled files listed in [Getting started](getting-started.md#use-a-release). Checksums remain in the ignored stage directory.

The release script requires a clean `master` tree, the Android result directory, and exactly one `Release X.Y.Z` commit since the previous tag. It checks both Maven coordinates, module metadata, AAR contents, the Android archive, documentation, invariants, iOS archive contents, architectures, and generated manifest before creating the package commit or tag.

Android coordinates are `com.noma4i:medialayer:X.Y.Z` and `com.noma4i:medialayer-ui:X.Y.Z`. The Maven branch contains `com/noma4i/medialayer/X.Y.Z/` and `com/noma4i/medialayer-ui/X.Y.Z/` at its root. Publication uses local Maven staging and does not need credentials.

The root `Package.swift` uses the release URL and checksum for SwiftPM. Keep `MediaLayer/Package.swift` and `scripts/build_ios.sh` for local development.

## Artifacts branch

Every release also publishes the four release files to the orphan branch `artifacts`: `MediaLayerFFI.xcframework.zip`, `medialayer.swift`, `medialayer-gen-macos-universal`, and `medialayer-android.zip`. The files are the same bytes as the GitHub Release attachments. The script checks each file against `SHA256SUMS` of the stage and fails when a file is missing, above 100 MB, or has a different hash.

Each version lives at `X.Y.Z/<file name>`. Each release adds one commit `Artifacts X.Y.Z` and keeps earlier versions. A repeat run with equal bytes pushes nothing. A repeat run with different bytes fails.

V3 and Bitrise read the files with a plain git clone, which needs only an SSH key with repository access:

```sh
git clone --branch artifacts --depth 1 git@github.com:noma4i/medialayer.git artifacts
ls artifacts/X.Y.Z/
```

To publish the branch for an already built stage `target/release-X.Y.Z`:

```sh
make release-artifacts VERSION=X.Y.Z            # dry run: builds the commit in a throwaway clone and prints the tree
make release-artifacts VERSION=X.Y.Z MODE=push  # pushes the branch
```

Optional arguments are `STAGE=<dir>` for another stage directory and `REMOTE=<url>` for another remote. `make release-publish` runs the same step before the GitHub Release page, so a failed push stops the release before the page is created.

## Tag type

The release tag is a lightweight tag on the `Package release X.Y.Z` commit. An annotated tag makes `git clone --branch X.Y.Z --depth 1` print `refs/tags/X.Y.Z ... is not a commit!`.
