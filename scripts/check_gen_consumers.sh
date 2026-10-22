#!/usr/bin/env bash
# check_gen_consumers.sh swift | kotlin-run <command...>: compiles gen/fixtures/Consumer.* against the source the generator emits and the built library.
# kotlin-run clears the Gradle test sources of the consumer, places the freshly generated ones, runs the command (the gradle-test step on brick)
# and removes them again, so a Gradle run outside this script never compiles stale generated files.
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
  kotlin-run)
    shift
    [ "$#" -gt 0 ] || { echo "usage: check_gen_consumers.sh kotlin-run <command...>" >&2; exit 2; }
    directory="$root/kotlin/src/test/kotlin/app/media"
    rm -rf "$directory"
    trap 'rm -rf "$directory"' EXIT
    trap 'exit 143' TERM INT
    cargo test --manifest-path "$root/gen/Cargo.toml" --features compile-kotlin compile_tests
    "$@"
    ;;
  *) echo "usage: check_gen_consumers.sh <swift | kotlin-run <command...>>" >&2; exit 2 ;;
esac
