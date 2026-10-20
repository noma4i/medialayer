#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(sysctl -n hw.ncpu)}" RUST_TEST_THREADS="${RUST_TEST_THREADS:-$(sysctl -n hw.ncpu)}"
bridge="$root/MediaLayer"
build="$root/target/ios-build"
targets=(aarch64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim)
case "${1:-}" in
  --check) targets=(aarch64-apple-darwin aarch64-apple-ios-sim) ;;
  --ios) targets=(aarch64-apple-ios aarch64-apple-ios-sim) ;;
  --device) targets=(aarch64-apple-ios) ;;
  "") ;;
  *) echo "usage: build_ios.sh [--check|--ios|--device]" >&2; exit 2 ;;
esac

for target in "${targets[@]}"; do
  cargo build --release --manifest-path "$root/core/Cargo.toml" --target "$target"
done

cargo build --manifest-path "$root/bindgen/Cargo.toml" --bin uniffi-bindgen
rm -rf "$build" "$bridge/MediaLayerFFI.xcframework"
mkdir -p "$build/headers/medialayerFFI" "$bridge/Sources/MediaLayer"

(cd -- "$root/core" && "$root/target/debug/uniffi-bindgen" generate \
  --library "$root/target/${targets[0]}/release/libmedialayer.a" --language swift --out-dir "$build/swift")

python3 "$root/scripts/swift_cancellation.py" "$build/swift/medialayer.swift"
cp "$build/swift/medialayerFFI.h" "$build/headers/medialayerFFI/"
cp "$build/swift/medialayerFFI.modulemap" "$build/headers/medialayerFFI/module.modulemap"
cp "$build/swift/medialayer.swift" "$bridge/Sources/MediaLayer/medialayer.swift"

arguments=()
for target in "${targets[@]}"; do
  arguments+=(-library "$root/target/$target/release/libmedialayer.a" -headers "$build/headers")
done
xcodebuild -create-xcframework "${arguments[@]}" -output "$bridge/MediaLayerFFI.xcframework"
echo "xcframework: $bridge/MediaLayerFFI.xcframework"
