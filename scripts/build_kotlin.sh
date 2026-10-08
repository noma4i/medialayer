#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$(ls -d "$HOME"/Library/Android/sdk/ndk/* | sort -V | tail -1)}"
module="$root/kotlin"

cargo build --manifest-path "$root/core/Cargo.toml" --features cli
rm -rf "$module/src/main/kotlin/com/noma4i/medialayer/medialayer.kt"
(cd -- "$root/core" && "$root/target/debug/uniffi-bindgen" generate \
  --library "$root/target/debug/libmedialayer.dylib" --language kotlin --no-format \
  --out-dir "$module/src/main/kotlin")

(cd -- "$root/core" && cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -t x86 \
  -o "$module/src/main/jniLibs" build --release)
find "$module/src/main/jniLibs" -name 'libmedialayer.so' | sort
