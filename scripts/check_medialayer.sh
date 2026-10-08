#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
gradle="${GRADLE:-$(ls -d "$HOME"/.gradle/wrapper/dists/gradle-9.*-bin/*/gradle-9.*/bin/gradle | sort -V | tail -1)}"

echo "== rust: cargo test"
cargo test --manifest-path "$root/core/Cargo.toml"

echo "== ios: xcframework, swift test"
bash "$root/scripts/build_ios.sh"
swift test --package-path "$root/swift"

echo "== kotlin: libs for 4 Android ABI, bindings, gradle unit tests"
bash "$root/scripts/build_kotlin.sh"
"$gradle" -p "$root/kotlin" testDebugUnitTest --console=plain

echo "check_medialayer: ok"
