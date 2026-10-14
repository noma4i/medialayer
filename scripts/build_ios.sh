#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(sysctl -n hw.ncpu)}" RUST_TEST_THREADS="${RUST_TEST_THREADS:-$(sysctl -n hw.ncpu)}"
bridge="$root/MediaLayer"
build="$root/target/ios-build"
targets=(aarch64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios)
check=0
case "${1:-}" in
  --check) check=1; targets=(aarch64-apple-darwin aarch64-apple-ios-sim) ;;
  "") ;;
  *) echo "usage: build_ios.sh [--check]" >&2; exit 2 ;;
esac

for target in "${targets[@]}"; do
  cargo build --release --manifest-path "$root/core/Cargo.toml" --target "$target"
done

cargo build --manifest-path "$root/core/Cargo.toml" --features cli
rm -rf "$build" "$bridge/MediaLayerFFI.xcframework"
mkdir -p "$build/headers/medialayerFFI" "$bridge/Sources/MediaLayer"

(cd -- "$root/core" && "$root/target/debug/uniffi-bindgen" generate \
  --library "$root/target/debug/libmedialayer.dylib" --language swift --out-dir "$build/swift")

cp "$build/swift/medialayerFFI.h" "$build/headers/medialayerFFI/"
cp "$build/swift/medialayerFFI.modulemap" "$build/headers/medialayerFFI/module.modulemap"
cp "$build/swift/medialayer.swift" "$bridge/Sources/MediaLayer/medialayer.swift"

mkdir -p "$build/ios-sim"
if [ "$check" = 1 ]; then
  cp "$root/target/aarch64-apple-ios-sim/release/libmedialayer.a" "$build/ios-sim/libmedialayer.a"
  libraries=("$root/target/aarch64-apple-darwin/release/libmedialayer.a" "$build/ios-sim/libmedialayer.a")
else
  lipo -create "$root/target/aarch64-apple-ios-sim/release/libmedialayer.a" "$root/target/x86_64-apple-ios/release/libmedialayer.a" \
    -output "$build/ios-sim/libmedialayer.a"
  libraries=("$root/target/aarch64-apple-darwin/release/libmedialayer.a" "$root/target/aarch64-apple-ios/release/libmedialayer.a" "$build/ios-sim/libmedialayer.a")
fi

arguments=()
for library in "${libraries[@]}"; do
  arguments+=(-library "$library" -headers "$build/headers")
done
xcodebuild -create-xcframework "${arguments[@]}" -output "$bridge/MediaLayerFFI.xcframework"
echo "xcframework: $bridge/MediaLayerFFI.xcframework"
