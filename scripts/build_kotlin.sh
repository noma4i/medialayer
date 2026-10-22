#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(getconf _NPROCESSORS_ONLN)}" RUST_TEST_THREADS="${RUST_TEST_THREADS:-$(getconf _NPROCESSORS_ONLN)}"
# shellcheck source=cargo_artifact.sh
source "$root/scripts/cargo_artifact.sh"
module="$root/kotlin"
part="${1:-all}"
case "$part" in --host | --abis | --release | all) ;; *) echo "usage: build_kotlin.sh [--host | --abis | --release]" >&2; exit 2 ;; esac

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
  bindgen="$(cargo build --manifest-path "$root/bindgen/Cargo.toml" --bin uniffi-bindgen --message-format=json-render-diagnostics | reported_artifact bin uniffi-bindgen)"
  library="$module/src/main/jniLibs/arm64-v8a/libmedialayer.so"
  if [ "$part" != --release ]; then
    library="$(cargo build --manifest-path "$root/core/Cargo.toml" --lib --message-format=json-render-diagnostics | reported_artifact cdylib medialayer)"
    dirname -- "$library" > "$module/host-library.path"
  fi
  (cd -- "$root/core" && "$bindgen" generate \
    --library "$library" --language kotlin --no-format \
    --out-dir "$module/src/main/kotlin")
fi
