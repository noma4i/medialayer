#!/usr/bin/env bash
# The whole MediaLayer check. Cheap checks run first, then the cargo, swift, kotlin and ship lanes one after another: the check
# runs one bounded command at a time and the first red lane stops the run.
# Every step has a time limit and its process group is killed past it. --fast runs the cheap lane plus the lanes of
# the touched paths. --list prints the steps. --only <lane> runs one lane.
set -uo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
gradle="${GRADLE:-gradle}"
slot="bash $root/scripts/check_bounded.sh {limit}"

# shellcheck source=check_steps.sh
source "$root/scripts/check_steps.sh"

steps=(
  "cheap|zoo|120|*|bash scripts/check_zoo.sh"
  "cheap|docs|120|*|bash scripts/check_docs.sh"
  "cheap|invariants|120|*|bash scripts/check_invariants.sh"
  "cheap|build-modes|120|*|bash scripts/check_build_modes.sh"
  "cheap|release-evidence|120|*|bash scripts/check_release_evidence.sh"
  "cargo|cargo-test|900|core/ gen/ Cargo.|$slot cargo-step cargo test --workspace"
  "swift|ios-slices|600|MediaLayer/ core/ gen/ Cargo.|$slot bash-step bash scripts/build_ios.sh --check"
  "swift|swift-test|900|MediaLayer/ core/ gen/ Cargo.|$slot bash-step bash scripts/check_swift_test.sh \"\" -derivedDataPath $root/target/api-derived-data.noindex"
  "swift|api-schema-swift|120|MediaLayer/ core/ gen/ Cargo.|bash scripts/api_schema.sh swift"
  "swift|doc-fences-swift|300|README.md docs/ scripts/doc_fences.py scripts/doc-fences/ MediaLayer/ core/ gen/ Cargo.|$slot bash-step python3 scripts/doc_fences.py swift"
  "swift|gen-consumer-swift|600|MediaLayer/ core/ gen/ Cargo.|$slot bash-step bash scripts/check_gen_consumers.sh swift"
  "kotlin|kotlin-bindings|600|kotlin/ core/ gen/ Cargo.|$slot bash-step bash scripts/build_kotlin.sh --host"
  "kotlin|gradle-test|1500|kotlin/ core/ gen/ Cargo.|$slot gradle-step bash scripts/check_gen_consumers.sh kotlin-run $gradle -p kotlin testDebugUnitTest --console=plain --no-watch-fs -Dorg.gradle.daemon.idletimeout=600000 -Dorg.gradle.jvmargs=\"-Xmx3g -XX:ErrorFile=$root/target/check-owner-\$CHECK_OWNER_ID-%p.log\" -Pkotlin.compiler.execution.strategy=in-process"
  "kotlin|api-schema-kotlin|600|kotlin/ core/ gen/ Cargo.|$slot gradle-step bash scripts/api_schema.sh kotlin"
  "kotlin|doc-fences-kotlin|600|README.md docs/ scripts/doc_fences.py scripts/doc-fences/ kotlin/ core/ gen/ Cargo.|$slot gradle-step python3 scripts/doc_fences.py kotlin $gradle -p kotlin :ui:compileDebugUnitTestKotlin --console=plain --no-watch-fs -Dorg.gradle.daemon.idletimeout=600000 -Dorg.gradle.jvmargs=\"-Xmx3g -XX:ErrorFile=$root/target/check-owner-\$CHECK_OWNER_ID-%p.log\" -Pkotlin.compiler.execution.strategy=in-process"
  "ship|android-release|900|-|$slot bash-step bash scripts/build_kotlin.sh --release"
  "ship|ios-device|600|-|$slot bash-step bash scripts/build_ios.sh --device"
)

select_lane_steps() {
  local lane="$1" allowed step step_lane
  case "$lane" in
    mac) allowed=' cheap cargo swift ' ;;
    brick) allowed=' kotlin ' ;;
    *) echo 'unknown report lane' >&2; return 2 ;;
  esac
  selected_steps=()
  for step in "${steps[@]}"; do
    step_lane="${step%%|*}"
    if [[ "$allowed" = *" $step_lane "* ]] || { [ "$lane" = mac ] && [[ "$step" = ship\|ios-device\|* ]]; }; then
      selected_steps+=("$step")
    fi
  done
}

if [ "${1:-}" = --list-lane ]; then
  select_lane_steps "${2:-}" || exit 2
  for step in "${selected_steps[@]}"; do
    IFS='|' read -r _ name _ paths _ <<< "$step"
    printf '%s|%s\n' "$name" "$paths"
  done
elif [ "${1:-}" = --report-lane ]; then
  # A step whose input hash equals the green record in the existing report is not run again; its record is carried over.
  [ "$#" -eq 4 ] && [ "$3" = --report ] || { echo 'usage: check_medialayer.sh --report-lane <mac|brick> --report <file>' >&2; exit 2; }
  lane="$2"
  report="$4"
  select_lane_steps "$lane" || exit 2
  source_tree="$(python3 "$root/scripts/release_evidence.py" tree)" || exit 1
  reusable="$(mktemp)"
  python3 "$root/scripts/release_evidence.py" reusable "$lane" "$report" > "$reusable" || exit 1
  passed=()
  pending=()
  for step in "${selected_steps[@]}"; do
    IFS='|' read -r _ name _ <<< "$step"
    if grep -qx -- "$name" "$reusable"; then
      echo "reused $name"
      passed+=("$name")
    else
      pending+=("$step")
    fi
  done
  rm -f "$reusable"
  code=0
  if [ "${#pending[@]}" -gt 0 ]; then
    steps=("${pending[@]}")
    output="$(mktemp)"
    run_checks | tee "$output"
    code="${PIPESTATUS[0]}"
    while read -r name; do passed+=("$name"); done < <(awk 'NF == 4 && $3 == "ok" && $4 ~ /^[0-9]+s$/ { print $2 }' "$output")
    rm -f "$output"
  fi
  python3 "$root/scripts/release_evidence.py" lane "$lane" "$report" "$source_tree" ${passed[@]+"${passed[@]}"}
  evidence_code=$?
  [ "$code" -eq 0 ] || exit "$code"
  exit "$evidence_code"
else
  run_checks "$@"
fi
