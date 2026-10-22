#!/usr/bin/env bash
# The whole MediaLayer check. Cheap checks run first, then the cargo, swift, kotlin and ship lanes one after another: the check
# holds one build slot command at a time and the first red lane stops the run.
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
  "cheap|docs|120|*|DOCS_STRICT=1 bash scripts/check_docs.sh"
  "cheap|invariants|120|*|bash scripts/check_invariants.sh"
  "cheap|invariants-selftest|120|*|bash scripts/check_invariants_selftest.sh"
  "cheap|gradle-guard-selftest|120|*|bash scripts/check_gradle_guard_selftest.sh"
  "cheap|build-modes|120|*|bash scripts/check_build_modes.sh"
  "cargo|cargo-test|900|core/ gen/ Cargo.|$slot cargo-step cargo test --workspace"
  "swift|ios-slices|600|MediaLayer/ core/ gen/ Cargo.|$slot bash-step bash scripts/build_ios.sh --check"
  "swift|swift-test|900|MediaLayer/ core/ gen/ Cargo.|$slot bash-step $HOME/.claude/bin/sim-run swift-test -- xcodebuild test -workspace $root/MediaLayer -scheme MediaLayer-Package -destination \"platform=iOS Simulator,id=@UDID@\" -derivedDataPath $root/.orca-artifacts/DD.noindex"
  "swift|gen-consumer-swift|600|MediaLayer/ core/ gen/ Cargo.|$slot bash-step bash scripts/check_gen_consumers.sh swift"
  "kotlin|kotlin-bindings|600|kotlin/ core/ gen/ Cargo.|$slot bash-step bash scripts/build_kotlin.sh --host"
  "kotlin|gen-consumer-kotlin|600|kotlin/ core/ gen/ Cargo.|$slot bash-step bash scripts/check_gen_consumers.sh kotlin"
  "kotlin|gradle-test|900|kotlin/ core/ gen/ Cargo.|$slot gradle-step $gradle -p kotlin testDebugUnitTest --console=plain --no-watch-fs -Dorg.gradle.daemon.idletimeout=600000 -Dorg.gradle.jvmargs=\"-Xmx3g -XX:ErrorFile=/tmp/check-owner-\$CHECK_OWNER_ID-%p.log\" -Pkotlin.compiler.execution.strategy=in-process"
  "ship|android-release|900|-|$slot bash-step bash scripts/build_kotlin.sh --release"
  "ship|ios-device|600|-|$slot bash-step bash scripts/build_ios.sh --device"
)

run_checks "$@"
