#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
bridge="$root/swift"
build="$root/target/ios-build"
targets=(aarch64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios)

for target in "${targets[@]}"; do
  cargo build --release --manifest-path "$root/core/Cargo.toml" --target "$target"
done

cargo build --manifest-path "$root/core/Cargo.toml" --features cli
rm -rf "$build" "$bridge/MediaLayerFFI.xcframework"
mkdir -p "$build/headers" "$bridge/Sources/MediaLayer"

(cd -- "$root/core" && "$root/target/debug/uniffi-bindgen" generate \
  --library "$root/target/debug/libmedialayer.dylib" --language swift --out-dir "$build/swift")

cp "$build/swift/medialayerFFI.h" "$build/headers/"
cp "$build/swift/medialayerFFI.modulemap" "$build/headers/module.modulemap"
cp "$build/swift/medialayer.swift" "$bridge/Sources/MediaLayer/medialayer.swift"

mkdir -p "$build/ios-sim"
lipo -create "$root/target/aarch64-apple-ios-sim/release/libmedialayer.a" "$root/target/x86_64-apple-ios/release/libmedialayer.a" \
  -output "$build/ios-sim/libmedialayer.a"

arguments=()
for library in "$root/target/aarch64-apple-darwin/release/libmedialayer.a" "$root/target/aarch64-apple-ios/release/libmedialayer.a" "$build/ios-sim/libmedialayer.a"; do
  arguments+=(-library "$library" -headers "$build/headers")
done
xcodebuild -create-xcframework "${arguments[@]}" -output "$bridge/MediaLayerFFI.xcframework"
echo "xcframework: $bridge/MediaLayerFFI.xcframework"
