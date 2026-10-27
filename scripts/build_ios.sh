#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
source "$root/scripts/build_common.sh"
bridge="$root/MediaLayer"
build="$root/target/ios-build"
targets=(aarch64-apple-ios aarch64-apple-ios-sim)
case "${1:-}" in
  --check) targets=(aarch64-apple-ios-sim) ;;
  --ios) targets=(aarch64-apple-ios aarch64-apple-ios-sim) ;;
  --device) targets=(aarch64-apple-ios) ;;
  "") ;;
  *) echo "usage: build_ios.sh [--check|--ios|--device]" >&2; exit 2 ;;
esac

libraries=()
for target in "${targets[@]}"; do
  library="$(cargo build --release --manifest-path "$root/core/Cargo.toml" --target "$target" --message-format=json-render-diagnostics | reported_artifact staticlib medialayer)"
  libraries+=("$library")
done

bindgen="$(cargo build --manifest-path "$root/bindgen/Cargo.toml" --bin uniffi-bindgen --message-format=json-render-diagnostics | reported_artifact bin uniffi-bindgen)"
rm -rf "$build" "$bridge/MediaLayerFFI.xcframework"
mkdir -p "$build/headers/medialayerFFI" "$bridge/Sources/MediaLayer"

(cd -- "$root/core" && "$bindgen" generate \
  --library "${libraries[0]}" --language swift --out-dir "$build/swift")

python3 "$root/scripts/swift_cancellation.py" "$build/swift/medialayer.swift"
cp "$build/swift/medialayerFFI.h" "$build/headers/medialayerFFI/"
cp "$build/swift/medialayerFFI.modulemap" "$build/headers/medialayerFFI/module.modulemap"
cp "$build/swift/medialayer.swift" "$bridge/Sources/MediaLayer/medialayer.swift"

arguments=()
for library in "${libraries[@]}"; do
  arguments+=(-library "$library" -headers "$build/headers")
done
xcodebuild -create-xcframework "${arguments[@]}" -output "$bridge/MediaLayerFFI.xcframework"
echo "xcframework: $bridge/MediaLayerFFI.xcframework"
