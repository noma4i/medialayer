#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$root"
fail() { printf 'release: %s\n' "$*" >&2; exit 1; }
[ "$#" -ge 3 ] || fail 'usage: release.sh <version> [--publish] --android <result-dir>'
version="$1"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail 'version must be x.y.z'
publish=""
android_dir=""
shift
while [ "$#" -gt 0 ]; do
  case "$1" in
    --publish) [ -z "$publish" ] || fail 'duplicate --publish'; publish=--publish; shift ;;
    --android) [ -z "$android_dir" ] && [ "$#" -ge 2 ] || fail 'usage: release.sh <version> [--publish] --android <result-dir>'; android_dir="$2"; shift 2 ;;
    *) fail 'usage: release.sh <version> [--publish] --android <result-dir>' ;;
  esac
done
[ -n "$android_dir" ] || fail 'Android release result is required: pass --android <result-dir>'
android_dir="$(cd -- "$android_dir" && pwd -P)" || fail 'Android release result directory not found'
[ -z "$(git status --porcelain)" ] || fail 'working tree is not clean'
[ "$(git branch --show-current)" = master ] || fail 'branch must be master'
previous_tag="$(git describe --tags --abbrev=0 --exclude "$version")" || fail 'previous tag not found'
subjects="$(git log --format=%s "$previous_tag"..HEAD)" || fail 'cannot read commits since previous tag'
[ "$(printf '%s\n' "$subjects" | grep -Fxc "Release $version" || true)" = 1 ] || fail "expected exactly one Release $version commit since $previous_tag"
[ "$(sed -n '/^\[package\]/,/^\[/s/^version = "\([^"]*\)"$/\1/p' core/Cargo.toml)" = "$version" ] || fail 'core version does not match'
grep -Fq "## $version - " CHANGELOG.md || fail 'changelog heading missing'
for check in check_docs check_zoo; do bash "scripts/$check.sh" || fail "$check failed"; done

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
android_archive="$android_dir/medialayer-android.zip"
[ -s "$android_archive" ] || fail 'Android release archive is missing'
for artifact in medialayer medialayer-ui medialayer-testing; do
  artifact_dir="$android_dir/maven/com/noma4i/$artifact/$version"
  for extension in aar pom module; do
    [ -s "$artifact_dir/$artifact-$version.$extension" ] || fail "Android Maven $artifact $extension is missing"
  done
done
mkdir "$tmp/android-archive"
(cd "$tmp/android-archive" && unzip -q "$android_archive")
for entry in \
  src/main/jniLibs/arm64-v8a/libmedialayer.so \
  src/main/jniLibs/armeabi-v7a/libmedialayer.so \
  src/main/jniLibs/x86_64/libmedialayer.so \
  src/main/jniLibs/x86/libmedialayer.so \
  src/main/kotlin/com/noma4i/medialayer/medialayer.kt; do
  [ -s "$tmp/android-archive/$entry" ] || fail "Android archive missing $entry"
done
mkdir "$tmp/android-aar"
(cd "$tmp/android-aar" && unzip -q "$android_dir/maven/com/noma4i/medialayer/$version/medialayer-$version.aar")
for abi in arm64-v8a armeabi-v7a x86_64 x86; do [ -s "$tmp/android-aar/jni/$abi/libmedialayer.so" ] || fail "medialayer AAR missing ABI $abi"; done
[ -s "$tmp/android-aar/classes.jar" ] || fail 'medialayer AAR missing classes.jar'
unzip -Z1 "$tmp/android-aar/classes.jar" > "$tmp/android-classes.list"
grep -Eq '^com/noma4i/medialayer/[^/]+\.class$' "$tmp/android-classes.list" || fail 'medialayer AAR missing Kotlin classes'
grep -Fq '<artifactId>medialayer</artifactId>' "$android_dir/maven/com/noma4i/medialayer-ui/$version/medialayer-ui-$version.pom" || fail 'medialayer-ui POM missing medialayer dependency'
grep -Fq '<artifactId>medialayer</artifactId>' "$android_dir/maven/com/noma4i/medialayer-testing/$version/medialayer-testing-$version.pom" || fail 'medialayer-testing POM missing medialayer dependency'
printf 'Android artifacts checked: %s\n' "$android_dir"

bash scripts/build_ios.sh

stage="$root/target/release-$version"
attachments=(MediaLayerFFI.xcframework.zip medialayer.swift medialayer-android.zip medialayer-gen-macos-universal)
if [ -d "$stage" ]; then
  for file in "${attachments[@]}" SHA256SUMS; do [ -s "$stage/$file" ] || fail "incomplete existing output: $stage/$file"; done
  (cd "$stage" && shasum -a 256 -c SHA256SUMS)
else
  mkdir -p "$stage"
  cp "$android_archive" "$stage/medialayer-android.zip"
  (cd MediaLayer && zip -qry "$stage/MediaLayerFFI.xcframework.zip" MediaLayerFFI.xcframework)
  cp MediaLayer/Sources/MediaLayer/medialayer.swift "$stage/medialayer.swift"
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release --manifest-path gen/Cargo.toml --target "$target"
  done
  lipo -create target/{aarch64-apple-darwin,x86_64-apple-darwin}/release/medialayer-gen -output "$stage/medialayer-gen-macos-universal"
  chmod +x "$stage/medialayer-gen-macos-universal"
  (cd "$stage" && shasum -a 256 "${attachments[@]}" > SHA256SUMS)
fi

mkdir -p "$tmp/archive"
unzip -q "$stage/MediaLayerFFI.xcframework.zip" -d "$tmp/archive"
archive_entries="$(unzip -Z1 "$stage/MediaLayerFFI.xcframework.zip")"
grep -Fxq 'MediaLayerFFI.xcframework/Info.plist' <<< "$archive_entries" || fail 'xcframework archive is missing Info.plist'
grep -Fq 'MediaLayerFFI.xcframework/ios-arm64/' <<< "$archive_entries" || fail 'xcframework archive is missing the iOS device slice'
grep -Fq 'MediaLayerFFI.xcframework/ios-arm64-simulator/' <<< "$archive_entries" || fail 'xcframework archive is missing the iOS simulator slice'
macos_slice="$(python3 - "$tmp/archive/MediaLayerFFI.xcframework/Info.plist" <<'PY'
import plistlib
import sys

with open(sys.argv[1], "rb") as manifest:
    libraries = plistlib.load(manifest)["AvailableLibraries"]
matches = [
    item["LibraryIdentifier"]
    for item in libraries
    if item.get("SupportedPlatform") == "macos"
    and "arm64" in item.get("SupportedArchitectures", [])
]
if len(matches) != 1:
    raise SystemExit("xcframework archive must contain one macOS arm64 slice")
print(matches[0])
PY
)" || fail 'xcframework archive is missing the macOS arm64 slice'
grep -Fq "MediaLayerFFI.xcframework/$macos_slice/" <<< "$archive_entries" || fail 'xcframework archive is missing the macOS slice'
check_archs() {
  local file="$1" expected="$2" actual
  actual="$(lipo -archs "$file" | tr ' ' '\n' | LC_ALL=C sort | tr '\n' ' ' | sed 's/ $//')"
  [ "$actual" = "$expected" ] || fail "unexpected architectures in $file: $actual"
}
device_binary="$tmp/archive/MediaLayerFFI.xcframework/ios-arm64/libmedialayer.a"
simulator_binary="$tmp/archive/MediaLayerFFI.xcframework/ios-arm64-simulator/libmedialayer.a"
macos_binary="$tmp/archive/MediaLayerFFI.xcframework/$macos_slice/libmedialayer.a"
[ -s "$device_binary" ] || fail 'xcframework archive is missing the device library'
[ -s "$simulator_binary" ] || fail 'xcframework archive is missing the simulator library'
[ -s "$macos_binary" ] || fail 'xcframework archive is missing the macOS library'
check_archs "$device_binary" 'arm64'
check_archs "$simulator_binary" 'arm64'
check_archs "$macos_binary" 'arm64'
check_archs "$stage/medialayer-gen-macos-universal" 'arm64 x86_64'
for file in "${attachments[@]}"; do [ -s "$stage/$file" ] || fail "missing release attachment: $file"; done
checksum="$(swift package compute-checksum "$stage/MediaLayerFFI.xcframework.zip")"

write_package() {
  cat <<EOF
// swift-tools-version: 6.0

import PackageDescription

let package = Package(
  name: "MediaLayer",
  platforms: [.iOS(.v17), .macOS(.v14)],
  products: [
    .library(name: "MediaLayer", targets: ["MediaLayer"]),
    .library(name: "MediaLayerUI", targets: ["MediaLayerUI"]),
    .library(name: "MediaLayerTesting", targets: ["MediaLayerTesting"])
  ],
  dependencies: [
    .package(url: "https://github.com/kean/Nuke", from: "13.2.0")
  ],
  targets: [
    .binaryTarget(
      name: "medialayerFFI",
      url: "https://github.com/noma4i/medialayer/releases/download/$version/MediaLayerFFI.xcframework.zip",
      checksum: "$checksum"
    ),
    .target(
      name: "MediaLayer",
      dependencies: ["medialayerFFI"],
      path: "MediaLayer/Sources/MediaLayer",
      swiftSettings: [.swiftLanguageMode(.v5)]
    ),
    .target(
      name: "MediaLayerUI",
      dependencies: ["MediaLayer", .product(name: "Nuke", package: "Nuke")],
      path: "MediaLayer/Sources/MediaLayerUI",
      swiftSettings: [.swiftLanguageMode(.v5)]
    ),
    .target(
      name: "MediaLayerTesting",
      dependencies: ["MediaLayer"],
      path: "MediaLayer/Sources/MediaLayerTesting",
      swiftSettings: [.swiftLanguageMode(.v5)]
    )
  ]
)
EOF
}
write_package > "$tmp/Package.swift"
mkdir -p "$tmp/package/MediaLayer/Sources/MediaLayer" "$tmp/package/MediaLayer/Sources/MediaLayerUI" "$tmp/package/MediaLayer/Sources/MediaLayerTesting"
cp "$tmp/Package.swift" "$tmp/package/Package.swift"
cp MediaLayer/Sources/MediaLayer/*.swift "$tmp/package/MediaLayer/Sources/MediaLayer/"
cp MediaLayer/Sources/MediaLayerUI/*.swift "$tmp/package/MediaLayer/Sources/MediaLayerUI/"
cp MediaLayer/Sources/MediaLayerTesting/*.swift "$tmp/package/MediaLayer/Sources/MediaLayerTesting/"
swift package --package-path "$tmp/package" dump-package > "$tmp/package.json"

notes="$stage/NOTES.md"
awk -v heading="## $version - " 'index($0, heading)==1 {found=1; next} found && /^## / {exit} found {print}' CHANGELOG.md > "$notes"
[ -s "$notes" ] || fail 'release notes are empty'
stage_files="$(find "$stage" -maxdepth 1 -type f -exec basename {} \; | LC_ALL=C sort)"
expected_stage_files="$(printf '%s\n' "${attachments[@]}" SHA256SUMS NOTES.md | LC_ALL=C sort)"
[ "$stage_files" = "$expected_stage_files" ] || fail "unexpected files in release stage: $stage_files"

maven_checkout="$tmp/maven-branch"
maven_remote=""
if [ "$publish" = --publish ]; then
  maven_remote="$(git remote get-url origin)"
  if [ -n "$(git ls-remote --heads "$maven_remote" maven | awk 'NR == 1 {print $1}')" ]; then
    git clone --branch maven --single-branch "$maven_remote" "$maven_checkout"
  else
    git -C "$tmp" init -b maven maven-branch
    git -C "$maven_checkout" remote add origin "$maven_remote"
  fi
  for artifact in medialayer medialayer-ui medialayer-testing; do
    source_dir="$android_dir/maven/com/noma4i/$artifact/$version"
    destination_dir="$maven_checkout/com/noma4i/$artifact/$version"
    if [ -d "$destination_dir" ]; then
      diff -qr "$source_dir" "$destination_dir" || fail "Maven $artifact $version already exists with different files"
    else
      mkdir -p "$(dirname "$destination_dir")"
      cp -R "$source_dir" "$destination_dir"
      git -C "$maven_checkout" add "com/noma4i/$artifact/$version"
    fi
  done
  if ! git -C "$maven_checkout" diff --cached --quiet; then
    git -C "$maven_checkout" commit -m "Maven $version"
  fi
fi

if [ "$publish" = --publish ]; then
  package_commits="$(git log --format=%s "$previous_tag"..HEAD | grep -Fxc "Package release $version" || true)"
  if [ "$package_commits" = 0 ]; then
    cp "$tmp/Package.swift" Package.swift
    git add Package.swift
    git add -f MediaLayer/Sources/MediaLayer/medialayer.swift
    git commit -m "Package release $version"
  elif [ "$package_commits" = 1 ] && [ "$(git log -1 --format=%s)" = "Package release $version" ]; then
    git show HEAD:Package.swift > "$tmp/committed-Package.swift"
    git show HEAD:MediaLayer/Sources/MediaLayer/medialayer.swift > "$tmp/committed-medialayer.swift"
    cmp -s "$tmp/Package.swift" "$tmp/committed-Package.swift" || fail 'existing Package.swift differs from the release package'
    cmp -s "$stage/medialayer.swift" "$tmp/committed-medialayer.swift" || fail 'existing medialayer.swift differs from the release package'
  else
    fail "unexpected Package release $version commit state"
  fi
  git tag "$version" 2>/dev/null || {
    [ "$(git rev-parse "$version^{commit}")" = "$(git rev-parse HEAD)" ] || fail "tag $version already points elsewhere"
  }
else
  printf '%s\n' 'Package.swift to be written:'
  cat "$tmp/Package.swift"
  printf 'Dry run: would write Package release %s, tag %s, push master and tag, publish Maven artifacts, and publish 4 attachments.\n' "$version" "$version"
fi

printf 'release artifacts: %s\n' "$stage"
if [ "$publish" = --publish ]; then
  git push origin master
  git push origin "$version"
  git -C "$maven_checkout" push -u origin maven
  if ! gh release view "$version" >/dev/null 2>&1; then
    gh release create "$version" --title "$version" --notes-file "$notes" "${attachments[@]/#/$stage/}"
  fi
  gh release view "$version" --json assets --jq '.assets[].name' > "$tmp/assets"
  for file in "${attachments[@]}"; do grep -Fxq "$file" "$tmp/assets" || fail "missing published attachment: $file"; done
  mkdir "$tmp/download"
  gh release download "$version" --dir "$tmp/download"
  (cd "$tmp/download" && shasum -a 256 -c "$stage/SHA256SUMS")
fi
