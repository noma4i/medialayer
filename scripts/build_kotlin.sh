#!/usr/bin/env bash
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=gradle_host_guard.sh
source "$here/gradle_host_guard.sh"
refuse_gradle_on_darwin build_kotlin.sh || exit $?

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(nproc)}" RUST_TEST_THREADS="${RUST_TEST_THREADS:-$(nproc)}"
module="$root/kotlin"
part="${1:-all}"
case "$part" in --host | --abis | --release | all) ;; *) echo "usage: build_kotlin.sh [--host | --abis | --release]" >&2; exit 2 ;; esac

if [ "$part" = --host ] || [ "$part" = all ]; then
  cargo build --manifest-path "$root/core/Cargo.toml" --lib
fi

if [ "$part" != --host ]; then
  abis=(-t arm64-v8a -t armeabi-v7a -t x86_64 -t x86)
  if [ "$part" = --release ]; then
    abis=(-t arm64-v8a -t armeabi-v7a)
    rm -rf "$module/src/main/jniLibs/x86" "$module/src/main/jniLibs/x86_64"
  fi
  (cd -- "$root/core" && cargo ndk "${abis[@]}" \
    -o "$module/src/main/jniLibs" build --release)
  find "$module/src/main/jniLibs" -name 'libmedialayer.so' | sort
fi

if [ "$part" != --abis ]; then
  cargo build --manifest-path "$root/bindgen/Cargo.toml" --bin uniffi-bindgen
  library="$root/target/debug/libmedialayer.so"
  [ "$part" != --release ] || library="$module/src/main/jniLibs/arm64-v8a/libmedialayer.so"
  (cd -- "$root/core" && "$root/target/debug/uniffi-bindgen" generate \
    --library "$library" --language kotlin --no-format \
    --out-dir "$module/src/main/kotlin")
fi
