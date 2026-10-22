#!/usr/bin/env bash
# check_gen_consumers.sh <swift|kotlin>: compiles gen/fixtures/Consumer.* against the source the generator emits and the built library.
# Kotlin names KOTLINC and KOTLIN_CHECK_CLASSPATH itself and runs on brick; a missing one fails the step.
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

case "${1:-}" in
  swift)
    swift build --package-path "$root/MediaLayer" --product MediaLayer > /dev/null
    bin="$(swift build --package-path "$root/MediaLayer" --show-bin-path)"
    SWIFTC="$(xcrun -f swiftc)"
    SWIFT_CHECK_ARGS="$(printf '["-sdk","%s","-I","%s","-I","%s/MediaLayer/MediaLayerFFI.xcframework/macos-arm64/Headers/medialayerFFI"]' "$(xcrun --sdk macosx --show-sdk-path)" "$bin" "$root")"
    export SWIFTC SWIFT_CHECK_ARGS
    cargo test --manifest-path "$root/gen/Cargo.toml" --features compile-swift compile_tests
    ;;
  kotlin) cargo test --manifest-path "$root/gen/Cargo.toml" --features compile-kotlin compile_tests ;;
  *) echo "usage: check_gen_consumers.sh <swift|kotlin>" >&2; exit 2 ;;
esac
