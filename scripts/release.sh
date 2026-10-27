#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$root"
fail() { printf 'release: %s\n' "$*" >&2; exit 1; }
[ "$#" -ge 1 ] && [ "$#" -le 2 ] || fail 'usage: release.sh <version> [--publish]'
version="$1"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail 'version must be x.y.z'
publish="${2:-}"
[ -z "$publish" ] || [ "$publish" = --publish ] || fail 'usage: release.sh <version> [--publish]'
[ -z "$(git status --porcelain)" ] || fail 'working tree is not clean'
[ "$(git branch --show-current)" = master ] || fail 'branch must be master'
previous_tag="$(git describe --tags --abbrev=0 --exclude "$version")" || fail 'previous tag not found'
subjects="$(git log --format=%s "$previous_tag"..HEAD)" || fail 'cannot read commits since previous tag'
[ "$(printf '%s\n' "$subjects" | grep -Fxc "Release $version" || true)" = 1 ] || fail "expected exactly one Release $version commit since $previous_tag"
[ "$(sed -n '/^\[package\]/,/^\[/s/^version = "\([^"]*\)"$/\1/p' core/Cargo.toml)" = "$version" ] || fail 'core version does not match'
grep -Fq "## $version - " CHANGELOG.md || fail 'changelog heading missing'
for check in check_docs check_invariants check_zoo; do bash "scripts/$check.sh" || fail "$check failed"; done
bash scripts/build_ios.sh

stage="$root/target/release-$version"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
attachments=(MediaLayerFFI.xcframework.zip medialayer.swift medialayer-gen-macos-universal)
if [ -d "$stage" ]; then
  for file in "${attachments[@]}" SHA256SUMS; do [ -s "$stage/$file" ] || fail "incomplete existing output: $stage/$file"; done
  (cd "$stage" && shasum -a 256 -c SHA256SUMS)
else
  mkdir -p "$stage"
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
check_archs() {
  local file="$1" expected="$2" actual
  actual="$(lipo -archs "$file" | tr ' ' '\n' | LC_ALL=C sort | tr '\n' ' ' | sed 's/ $//')"
  [ "$actual" = "$expected" ] || fail "unexpected architectures in $file: $actual"
}
device_binary="$tmp/archive/MediaLayerFFI.xcframework/ios-arm64/libmedialayer.a"
simulator_binary="$tmp/archive/MediaLayerFFI.xcframework/ios-arm64-simulator/libmedialayer.a"
[ -s "$device_binary" ] || fail 'xcframework archive is missing the device library'
[ -s "$simulator_binary" ] || fail 'xcframework archive is missing the simulator library'
check_archs "$device_binary" 'arm64'
check_archs "$simulator_binary" 'arm64'
check_archs "$stage/medialayer-gen-macos-universal" 'arm64 x86_64'
for file in "${attachments[@]}"; do [ -s "$stage/$file" ] || fail "missing release attachment: $file"; done
checksum="$(swift package compute-checksum "$stage/MediaLayerFFI.xcframework.zip")"

write_package() {
  cat <<EOF
// swift-tools-version: 6.0

import PackageDescription

let package = Package(
  name: "MediaLayer",
  platforms: [.iOS(.v17)],
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
  git tag -a "$version" -m "$version" 2>/dev/null || {
    [ "$(git rev-parse "$version^{commit}")" = "$(git rev-parse HEAD)" ] || fail "tag $version already points elsewhere"
  }
else
  printf '%s\n' 'Package.swift to be written:'
  cat "$tmp/Package.swift"
  printf 'Dry run: would write Package release %s, tag %s, push master and tag, and publish 3 attachments.\n' "$version" "$version"
fi

printf 'release artifacts: %s\n' "$stage"
if [ "$publish" = --publish ]; then
  git push origin master
  git push origin "$version"
  if ! gh release view "$version" >/dev/null 2>&1; then
    gh release create "$version" --title "$version" --notes-file "$notes" "${attachments[@]/#/$stage/}"
  fi
  gh release view "$version" --json assets --jq '.assets[].name' > "$tmp/assets"
  for file in "${attachments[@]}"; do grep -Fxq "$file" "$tmp/assets" || fail "missing published attachment: $file"; done
  mkdir "$tmp/download"
  gh release download "$version" --dir "$tmp/download"
  (cd "$tmp/download" && shasum -a 256 -c "$stage/SHA256SUMS")
fi
