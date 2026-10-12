#!/usr/bin/env bash
# The whole MediaLayer check. Cheap checks run first and stop everything on a failure, then the cargo, swift, kotlin
# and ship lanes run in parallel (every heavy command takes a build slot, so at most BUILD_SLOTS run at once).
# Every step has a time limit and its process group is killed past it. --fast runs the cheap lane plus the lanes of
# the touched paths. --list prints the steps. --only <lane> runs one lane.
set -uo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
gradle="${GRADLE:-$(ls -d "$HOME"/.gradle/wrapper/dists/gradle-9.*-bin/*/gradle-9.*/bin/gradle 2> /dev/null | sort -V | tail -1)}"
slot="$HOME/.claude/bin/build-slot -- bash $root/scripts/check_bounded.sh {limit}"
[ -x "$HOME/.claude/bin/build-slot" ] || slot="bash $root/scripts/check_bounded.sh {limit}"

# shellcheck source=check_steps.sh
source "$root/scripts/check_steps.sh"

steps=(
  "cheap|zoo|120|*|bash scripts/check_zoo.sh"
  "cheap|docs|120|*|bash scripts/check_docs.sh"
  "cheap|invariants|120|*|bash scripts/check_invariants.sh"
  "cheap|invariants-selftest|120|*|bash scripts/check_invariants_selftest.sh"
  "cargo|cargo-test|900|core/ gen/ Cargo.|$slot cargo-step cargo test --manifest-path core/Cargo.toml"
  "swift|ios-slices|600|MediaLayer/|$slot bash-step bash scripts/build_ios.sh --check"
  "swift|swift-test|900|MediaLayer/|$slot swift-step swift test --package-path MediaLayer"
  "kotlin|kotlin-bindings|600|kotlin/|$slot bash-step bash scripts/build_kotlin.sh --host"
  "kotlin|gradle-test|900|kotlin/|$slot gradle-step $gradle -p kotlin testDebugUnitTest --console=plain --no-watch-fs"
  "ship|android-abis|900|-|$slot bash-step bash scripts/build_kotlin.sh --abis"
  "ship|ios-device-check|600|-|$slot ios-check bash -c 'cargo check --manifest-path core/Cargo.toml --target aarch64-apple-ios && cargo check --manifest-path core/Cargo.toml --target x86_64-apple-ios'"
)

run_checks "$@"
