#!/usr/bin/env bash
# build_kotlin.sh [--host | --abis]: --host builds the debug library and the Kotlin bindings (what the unit tests
# load through JNA), --abis builds the 4 Android libraries. No argument does both.
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=gradle_host_guard.sh
source "$here/gradle_host_guard.sh"
refuse_gradle_on_darwin build_kotlin.sh || exit $?

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(sysctl -n hw.ncpu)}" RUST_TEST_THREADS="${RUST_TEST_THREADS:-$(sysctl -n hw.ncpu)}"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$(ls -d "$HOME"/Library/Android/sdk/ndk/* | sort -V | tail -1)}"
module="$root/kotlin"
part="${1:-all}"
case "$part" in --host | --abis | all) ;; *) echo "usage: build_kotlin.sh [--host | --abis]" >&2; exit 2 ;; esac

if [ "$part" != --abis ]; then
  cargo build --manifest-path "$root/core/Cargo.toml" --features cli
  rm -rf "$module/src/main/kotlin/com/noma4i/medialayer/medialayer.kt"
  (cd -- "$root/core" && "$root/target/debug/uniffi-bindgen" generate \
    --library "$root/target/debug/libmedialayer.dylib" --language kotlin --no-format \
    --out-dir "$module/src/main/kotlin")
fi

if [ "$part" != --host ]; then
  (cd -- "$root/core" && cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -t x86 \
    -o "$module/src/main/jniLibs" build --release)
  find "$module/src/main/jniLibs" -name 'libmedialayer.so' | sort
fi
