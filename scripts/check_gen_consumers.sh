#!/usr/bin/env bash
# check_gen_consumers.sh swift [<simulator UDID>] | kotlin-run <command...>: runs gen/fixtures/Consumer.* as tests on the source the generator emits and the built library.
# swift: the Rust test places the sources in the Swift package tests, runs check_swift_test.sh for them and removes them again.
# kotlin-run clears the Gradle test sources of the consumer, places the freshly generated ones, runs the command (the gradle-test step on brick)
# and removes them again, so a Gradle run outside this script never compiles stale generated files.
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

case "${1:-}" in
  swift)
    SWIFT_TEST_RUNNER=bash
    SWIFT_TEST_ARGS="$(printf '["%s/scripts/check_swift_test.sh","%s","-only-testing:MediaLayerTests/GeneratedSourcesTests"]' "$root" "${2:-}")"
    export SWIFT_TEST_RUNNER SWIFT_TEST_ARGS
    cargo test --manifest-path "$root/gen/Cargo.toml" --features compile-swift compile_tests -- --nocapture
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
  *) echo "usage: check_gen_consumers.sh <swift [udid] | kotlin-run <command...>>" >&2; exit 2 ;;
esac
