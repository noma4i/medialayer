#!/usr/bin/env bash
# Self test of the checks themselves: INV-CHECK-01..08. Each function builds a fixture repository, breaks it in
# one way and expects the matching check to go red. Runs in seconds.
set -uo pipefail
unset SPEC TEST_DIRS DOC_PATHS

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=check_steps.sh
source "$here/check_steps.sh"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failed=0

fixture() {
  repo="$work/repo"
  rm -rf "$repo"
  mkdir -p "$repo/scripts" "$repo/specs" "$repo/core/src"
  cp "$here/check_invariants.sh" "$repo/scripts/"
  cat > "$repo/specs/invariants.md" <<'SPEC'
| INV-T-01 | REQUIRED | f | s | c | I | break it | A |
| INV-T-02 | UNBUILT | f | s | c | I | break it | A; reason: owner is the refresh slice, the server push is missing |
SPEC
  cat > "$repo/core/src/t.rs" <<'RUST'
#[test]
fn inv_t_01_it_holds() {
    assert_eq!(1, 1);
}
RUST
}

run_check() { bash "$repo/scripts/check_invariants.sh" > "$work/out" 2>&1; }

expect_green() {
  run_check || { echo "FAIL expected green: $1"; cat "$work/out"; failed=1; }
}

expect_red() {
  if run_check; then echo "FAIL expected red: $1"; cat "$work/out"; failed=1; fi
}

inv_check_01_a_required_row_without_a_test_is_red() {
  fixture
  expect_green "fixture"
  sed -i '' 's/fn inv_t_01_it_holds/fn it_holds/' "$repo/core/src/t.rs"
  expect_red "test renamed"
}

inv_check_02_a_test_without_an_assertion_does_not_cover() {
  fixture
  printf '#[test]\nfn inv_t_01_it_holds() {\n    let _ = 1;\n}\n' > "$repo/core/src/t.rs"
  expect_red "empty body"
}

verdict_of() { awk -v id="$1" '$1 == id { $1 = ""; $2 = ""; $3 = ""; $4 = ""; sub(/^ +/, ""); print }' "$work/out"; }

expect_red_verdict() {
  [ "$(verdict_of "$1")" != covered ] || { echo "FAIL expected not covered: $1 ($2)"; failed=1; }
}

expect_green_verdict() {
  [ "$(verdict_of "$1")" = covered ] || { echo "FAIL expected covered: $1 ($2), verdict '$(verdict_of "$1")'"; failed=1; }
}

inv_check_02_comments_strings_ignored_tests_and_swift_expect_are_read_right() {
  fixture
  : > "$repo/specs/invariants.md"
  for n in 1 2 3 4 5 6; do
    echo "| INV-F-0$n | REQUIRED | f | s | c | I | break it | A |" >> "$repo/specs/invariants.md"
  done
  cat > "$repo/core/src/t.rs" <<'RUST'
#[test]
fn inv_f_01_comment() {
    let value = 1 + 1; // the old test used assert_eq here
}
#[test]
fn inv_f_02_string() {
    let text = "we assert nothing here";
}
/*
#[test]
fn inv_f_03_block() {
    assert_eq!(1, 1);
}
*/
#[test]
#[ignore]
fn inv_f_04_ignored() {
    assert_eq!(1, 1);
}
#[test]
fn inv_f_05_name() {
    let is_asserted = true;
}
RUST
  mkdir -p "$repo/swift/Tests"
  cat > "$repo/swift/Tests/S.swift" <<'SWIFT'
@Test func inv_f_06_swift() {
    #expect(1 == 1)
}
SWIFT
  run_check
  for n in 1 2 3 4 5; do expect_red_verdict "INV-F-0$n" "no real assertion"; done
  expect_green_verdict "INV-F-06" "Swift #expect on its own line"
}

inv_check_03_unbuilt_needs_a_reason_and_required_needs_a_mutation() {
  fixture
  sed -i '' 's/; reason: [^|]*//' "$repo/specs/invariants.md"
  expect_red "unbuilt without a reason"
  fixture
  sed -i '' '/INV-T-02/d; s/| break it |/|  |/' "$repo/specs/invariants.md"
  expect_red "required without a mutation"
}

inv_check_03_a_template_reason_is_not_a_reason() {
  fixture
  sed -i '' 's/reason: owner is the refresh slice, the server push is missing/reason: no test named inv_t_02_ yet, A26/' "$repo/specs/invariants.md"
  expect_red "reason restates the status with the id"
  fixture
  sed -i '' 's/reason: owner is the refresh slice, the server push is missing/reason: promised by the docs, no implementation/' "$repo/specs/invariants.md"
  expect_red "reason is the docs template"
  fixture
  sed -i '' 's/reason: owner is the refresh slice, the server push is missing/reason: owner is the refresh slice, the server push is missing/' "$repo/specs/invariants.md"
  expect_green "a reason with an owner"
}

inv_check_04_a_missing_registry_is_red() {
  fixture
  rm "$repo/specs/invariants.md"
  expect_red "registry removed"
}

expect_red_step_killed() {
  sleep 1
  if [ "$1" -ne 124 ] || pgrep -f 'sleep 601.7' > /dev/null; then
    echo "FAIL step code $1, leftover: $(pgrep -fl 'sleep 601.7' || true)"
    pkill -f 'sleep 601.7' || true
    failed=1
  fi
}

inv_check_05_a_step_past_its_limit_leaves_no_process() {
  local code=0
  bash "$here/check_bounded.sh" 2 selftest bash -c 'set -m; sleep 601.7 & wait' 2> /dev/null || code=$?
  expect_red_step_killed "$code"
}

engine() {
  root="$work/engine"
  mkdir -p "$root/target"
  rm -f "$root"/ran-*
  changed_files() { echo "$CHANGED"; }
  steps=(
    "cheap|c1|20|*|$c1"
    "core|k1|20|core/|touch ran-k1 && $k1"
    "swift|s1|20|swift/|touch ran-s1 && $s1"
    "final|f1|20|-|touch ran-f1 && $f1"
  )
  (run_checks "$@") > "$work/engine.out" 2>&1
}

expect_red_engine() {
  local label="$1"
  shift
  if engine "$@"; then echo "FAIL expected red: $label"; cat "$work/engine.out"; failed=1; fi
}

expect_green_engine() {
  local label="$1"
  shift
  engine "$@" || { echo "FAIL expected green: $label"; cat "$work/engine.out"; failed=1; }
}

inv_check_06_fast_and_full_fail_on_the_same_step_and_cheap_runs_first() {
  c1=true; k1=true; s1=true; f1=true; CHANGED="core/a.rs"
  expect_green_engine "all steps pass in full"
  [ -e "$work/engine/ran-f1" ] || { echo "FAIL expect_green: the final lane did not run"; failed=1; }
  f1=false
  expect_red_engine "final step fails in full"
  f1=true; k1=false
  expect_red_engine "lane step fails, final must not mask it"
  [ ! -e "$work/engine/ran-f1" ] || { echo "FAIL expect_red: final ran after a failed lane"; failed=1; }
  k1=true
  expect_green_engine "all steps pass again"
  expect_green_engine "all steps pass in fast" --fast
  c1=false
  expect_red_engine "cheap step fails in full"
  expect_red_engine "cheap step fails in fast" --fast
  [ -e "$work/engine/ran-k1" ] && { echo "FAIL expect_red: a build step ran after a cheap failure"; failed=1; }
  c1=true; k1=false
  expect_red_engine "core step fails in full"
  expect_red_engine "core step fails in fast" --fast
  k1=true; s1=false; CHANGED="swift/b.swift"
  expect_red_engine "swift step fails in full"
  expect_red_engine "swift step fails in fast" --fast
  local full fast_list
  full="$(engine --list; cat "$work/engine.out")"
  fast_list="$(engine --fast --list; cat "$work/engine.out")"
  [ -n "$fast_list" ] && [ -z "$(comm -13 <(echo "$full" | sort) <(echo "$fast_list" | sort))" ] \
    || { echo "FAIL expect_green: a fast step is not a full step"; failed=1; }
}

inv_check_06_lanes_run_one_after_another_and_the_first_red_lane_stops_the_run() {
  root="$work/engine"
  mkdir -p "$root/target"
  rm -f "$root"/overlap "$root"/ran-*
  rmdir "$root/lock" 2> /dev/null
  changed_files() { echo "$CHANGED"; }
  local hold='mkdir lock 2> /dev/null || touch overlap; sleep 1; rmdir lock 2> /dev/null; true'
  steps=("a|a1|20|-|$hold" "b|b1|20|-|$hold" "c|c1|20|-|touch ran-c1; false" "d|d1|20|-|touch ran-d1")
  (run_checks) > "$work/engine.out" 2>&1
  [ ! -e "$root/overlap" ] || { echo "FAIL expect_red: 2 lanes ran at the same time"; failed=1; }
  [ -e "$root/ran-c1" ] && [ ! -e "$root/ran-d1" ] || { echo "FAIL expect_red: a lane ran after the first red lane"; failed=1; }
}

expect_green_counter() {
  [ "$1" -ge "$(($2 > 1 ? $2 - 1 : 1))" ] || { echo "FAIL the counter printed $1 CPU seconds for a child that burned $2"; failed=1; }
}

inv_check_08_the_cpu_counter_sees_a_build_daemon() {
  root="$work/engine"
  mkdir -p "$root/target" "$work/jdk"
  rm -f "$work/marker-java"
  changed_files() { echo "$CHANGED"; }
  cp "$(command -v perl)" "$work/jdk/java"
  local child='while ((times)[0] + (times)[1] < 3) {} open(F, q(>), q(MARKER)); print F $$, q( ), int((times)[0] + (times)[1]); close(F); sleep 60' waiting='until [ -s MARKER ] && awk -v pid=$(cut -d" " -f1 MARKER) -v cpu=$(cut -d" " -f2 MARKER) "\$1 == pid && \$2 >= cpu - 1 { found = 1 } END { exit !found }" "$CHECK_SAMPLES_FILE"; do sleep 1; done'
  child="${child//MARKER/$work/marker-java}"
  waiting="${waiting//MARKER/$work/marker-java}"
  rm -f "$work/release"
  steps=("a|a1|280|-|$work/jdk/java -e 'if (!fork) { exec(q(perl), q(-e), q($child)) } until (-e q($work/release)) { sleep 1 }' & $waiting; touch $work/release; wait")
  (run_checks) > "$work/engine.out" 2>&1
  pkill -f "perl -e while .(times)" 2> /dev/null
  local counted
  counted="$(sed -n 's/^check: ok, at least \([0-9]*\) CPU seconds.*/\1/p' "$work/engine.out")"
  expect_green_counter "${counted:-0}" "$(cut -d' ' -f2 "$work/marker-java" 2> /dev/null || echo 0)"
}

expect_green_run_daemon() {
  [ "$1" -ge "$(($2 > 1 ? $2 - 1 : 1))" ] && [ "$1" -le $(($2 + 2)) ] || { echo "FAIL the counter printed $1 CPU seconds: the daemon of this run burned $2, the daemon of another run kept burning"; failed=1; }
}

inv_check_08_a_daemon_that_outlives_the_step_is_counted_and_another_runs_daemon_is_not() {
  root="$work/engine"
  mkdir -p "$root/target" "$work/jdk"
  rm -f "$work/marker-mine"
  changed_files() { echo "$CHANGED"; }
  cp "$(command -v perl)" "$work/jdk/java"
  local mine other waiting='until [ -s MARKER ] && awk -v pid=$(cut -d" " -f1 MARKER) -v cpu=$(cut -d" " -f2 MARKER) "\$1 == pid && \$2 >= cpu - 1 { found = 1 } END { exit !found }" "$CHECK_SAMPLES_FILE"; do sleep 1; done'
  mine='if (!fork) { exec(q(perl), q(-e), q(while ((times)[0] + (times)[1] < 3) {} open(F, q(>), q(MARKER)); print F $$, q( ), int((times)[0] + (times)[1]); close(F); sleep 60), q(GradleDaemon), q(-XX:ErrorFile=/tmp/check-owner-) . $ENV{CHECK_OWNER_ID} . q(-x.log)) }'
  other='if (!fork) { exec(q(perl), q(-e), q(1 while 1), q(GradleDaemon), q(-XX:ErrorFile=/tmp/check-owner-other-run-x.log)) }'
  mine="${mine//MARKER/$work/marker-mine}"
  waiting="${waiting//MARKER/$work/marker-mine}"
  steps=("a|a1|280|-|$work/jdk/java -e '$mine'; $work/jdk/java -e '$other'; $waiting")
  (run_checks) > "$work/engine.out" 2>&1
  pkill -f 'GradleDaemon -XX:ErrorFile=/tmp/check-owner-' 2> /dev/null
  local counted
  counted="$(sed -n 's/^check: ok, at least \([0-9]*\) CPU seconds.*/\1/p' "$work/engine.out")"
  expect_green_run_daemon "${counted:-0}" "$(cut -d' ' -f2 "$work/marker-mine" 2> /dev/null || echo 0)"
}

inv_check_07_a_name_without_a_row_is_red() {
  fixture
  mkdir -p "$repo/docs"
  echo "A doc without any rule id." > "$repo/docs/a.md"
  expect_green "doc names no row"
  echo "The rule INV-T-01 is checked." > "$repo/docs/a.md"
  expect_green "doc names an existing row"
  echo "The rule INV-T-09 is checked." > "$repo/docs/a.md"
  expect_red "doc names a row that does not exist"
}

table_repo() {
  repo="$work/table"
  rm -rf "$repo"
  mkdir -p "$repo/scripts"
  cp "$here/check_medialayer.sh" "$here/check_steps.sh" "$here/gradle_host_guard.sh" "$repo/scripts/"
  git -C "$repo" init -q -b master
  git -C "$repo" -c user.name=t -c user.email=t@t add -A
  git -C "$repo" -c user.name=t -c user.email=t@t commit -q -m base
}

listed() { bash "$repo/scripts/check_medialayer.sh" "$@" --list 2> /dev/null; }

expect_green_that() {
  local label="$1"
  shift
  "$@" || { echo "FAIL expected: $label"; failed=1; }
}

list_has() { grep -Eq -- "$1" "$work/list"; }

lanes_of_fast_list() { listed --fast | cut -d'|' -f1 | sort -u | paste -sd' ' -; }

fast_selects_all_platform_lanes() {
  local lanes
  lanes="$(lanes_of_fast_list)"
  [[ "$lanes" == *swift* && "$lanes" == *kotlin* && "$lanes" == *cargo* ]] || { echo "lanes: $lanes"; return 1; }
}

inv_check_09_a_core_or_gen_edit_selects_the_cargo_and_both_platform_lanes_and_the_crate_gen_tests_run() {
  table_repo
  listed > "$work/list"
  expect_green_that "the full cargo-test step runs the tests of the crate gen" list_has '^cargo\|cargo-test\|.*(--workspace|medialayer-gen)'
  local touched
  for touched in core/src/a.rs gen/src/a.rs; do
    mkdir -p "$repo/$(dirname "$touched")"
    echo x > "$repo/$touched"
    expect_green_that "fast selects cargo, swift and kotlin for $touched" fast_selects_all_platform_lanes
    rm -f "$repo/$touched"
  done
}

docs_repo() {
  repo="$work/docs"
  rm -rf "$repo"
  mkdir -p "$repo/scripts" "$repo/docs"
  cp "$here/check_docs.sh" "$repo/scripts/"
  echo "# comment" > "$repo/scripts/docs_unbuilt.txt"
  printf 'Intro.\n\n```swift\nlet a = 1\n```\n\n```kotlin\nval a = 1\n```\n' > "$repo/README.md"
}

docs_code() {
  local code=0
  "$@" bash "$repo/scripts/check_docs.sh" > "$work/out" 2>&1 || code=$?
  echo "$code" > "$work/code"
}

code_is() { [ "$(cat "$work/code")" = "$1" ]; }
code_is_not_zero() { [ "$(cat "$work/code")" != 0 ]; }
out_has_not_checked() { grep -q 'NOT CHECKED' "$work/out"; }
out_has_no_not_checked() { ! grep -q 'NOT CHECKED' "$work/out"; }

inv_check_10_docs_examples_without_generated_bindings_are_not_checked_and_strict_fails() {
  docs_repo
  docs_code env
  expect_green_that "a NOT CHECKED status without bindings" out_has_not_checked
  expect_green_that "code 0 outside strict mode" code_is 0
  docs_code env DOCS_STRICT=1
  expect_green_that "a nonzero code in strict mode without bindings" code_is_not_zero
  mkdir -p "$repo/MediaLayer/Sources/MediaLayer" "$repo/kotlin/src/main/kotlin"
  echo "public enum MediaState {}" > "$repo/MediaLayer/Sources/MediaLayer/medialayer.swift"
  echo "sealed class MediaState" > "$repo/kotlin/src/main/kotlin/medialayer.kt"
  docs_code env DOCS_STRICT=1
  expect_green_that "green in strict mode with bindings" code_is 0
  expect_green_that "no NOT CHECKED with bindings" out_has_no_not_checked
}

docs_step_is_strict() { grep -q 'DOCS_STRICT=1 bash scripts/check_docs.sh' "$here/check_medialayer.sh"; }

inv_check_11_the_full_check_compiles_the_generated_consumers_on_both_platforms() {
  table_repo
  listed > "$work/list"
  expect_green_that "a swift step that compiles the generated consumer" list_has '^swift\|.*check_gen_consumers.sh swift'
  expect_green_that "a kotlin step that compiles the generated consumer" list_has '^kotlin\|gradle-test\|.*check_gen_consumers.sh kotlin-run '
  expect_green_that "the docs step runs in strict mode" docs_step_is_strict
}

swift_test_runs() {
  local bin="$work/swift-bin" calls="$work/swift-calls"
  rm -rf "$bin" "$calls"
  mkdir -p "$bin"
  printf '#!/bin/sh\necho "xcodebuild $*" >> "%s"\n' "$calls" > "$bin/xcodebuild"
  printf '#!/bin/sh\nprintf "    iPhone 17 (11111111-2222-3333-4444-555555555555) (Shutdown)\\n"\n' > "$bin/xcrun"
  chmod +x "$bin"/*
  PATH="$bin:$PATH" bash "$here/check_swift_test.sh" "$@" > /dev/null 2>&1
}

swift_test_picks_a_simulator_without_an_argument() { swift_test_runs && grep -q 'xcodebuild test .*id=11111111-2222-3333-4444-555555555555' "$work/swift-calls"; }
swift_test_takes_a_udid_argument() { swift_test_runs AAAAAAAA-0000-0000-0000-000000000000 && grep -q 'xcodebuild test .*-destination platform=iOS Simulator,id=AAAAAAAA-0000-0000-0000-000000000000 ' "$work/swift-calls"; }
swift_test_takes_a_destination_argument() { swift_test_runs "platform=iOS Simulator,name=iPhone 17" && grep -q 'xcodebuild test .*-destination platform=iOS Simulator,name=iPhone 17 ' "$work/swift-calls"; }
swift_step_names_no_home_file() {
  listed | grep '^swift|swift-test|' > "$work/swift-step"
  ! grep -q 'HOME\|~/' "$work/swift-step"
}

inv_check_12_the_swift_test_lane_runs_on_a_machine_without_the_local_tooling() {
  table_repo
  expect_green_that "the swift-test script picks a simulator without an argument" swift_test_picks_a_simulator_without_an_argument
  expect_green_that "the swift-test script runs on the simulator UDID it is given" swift_test_takes_a_udid_argument
  expect_green_that "the swift-test script runs on the destination it is given" swift_test_takes_a_destination_argument
  expect_green_that "the swift-test step command names no file under HOME" swift_step_names_no_home_file
}

kotlin_run_repo() {
  repo="$work/kotlin-run"
  rm -rf "$repo"
  mkdir -p "$repo/scripts" "$repo/bin" "$repo/kotlin/src/test/kotlin/app/media"
  sed "/opt\/homebrew\/opt\/rustup/d" "$here/check_gen_consumers.sh" > "$repo/scripts/check_gen_consumers.sh"
  echo "class Stale" > "$repo/kotlin/src/test/kotlin/app/media/Stale.kt"
  printf '#!/bin/sh\nmkdir -p "%s/kotlin/src/test/kotlin/app/media"\necho "class Fresh" > "%s/kotlin/src/test/kotlin/app/media/Fresh.kt"\n' "$repo" "$repo" > "$repo/bin/cargo"
  chmod +x "$repo/bin/cargo"
}

inv_check_06_the_owner_id_does_not_depend_on_shasum() {
  changed_files() { echo "$CHANGED"; }
  steps=("a|a1|20|-|printf '%s' \"\$CHECK_OWNER_ID\" > owner")
  shasum() { echo "shasum: command not found" >&2; return 127; }
  owner_of() {
    root="$work/$1"
    mkdir -p "$root/target"
    rm -f "$root/owner"
    (run_checks) > "$work/engine.out" 2>&1
    cat "$root/owner" 2> /dev/null
  }
  local first second other
  first="$(owner_of engine)"
  second="$(owner_of engine)"
  other="$(owner_of engine-other)"
  [[ "$first" =~ ^[0-9a-f]{12}$ ]] && [ "$first" = "$second" ] && [ "$first" != "$other" ] \
    || { echo "FAIL expect_green: owner ids '$first' '$second' '$other' are not stable 12 hex per root"; failed=1; }
  ! grep -q 'command not found' "$work/engine.out" || { echo "FAIL expect_green: the run called a missing tool"; failed=1; }
  python3() { return 1; }
  root="$work/engine"
  rm -f "$root/owner"
  if (run_checks) > "$work/engine.out" 2>&1; then echo "FAIL expect_red: the run went on without an owner id"; failed=1; fi
  [ ! -e "$root/owner" ] || { echo "FAIL expect_red: a step ran without an owner id"; failed=1; }
  unset -f shasum python3 owner_of
}

kotlin_run_compiles_only_fresh_sources() {
  PATH="$repo/bin:$PATH" bash "$repo/scripts/check_gen_consumers.sh" kotlin-run sh -c 'ls "$1" > "$2"' _ "$repo/kotlin/src/test/kotlin/app/media" "$work/seen" > /dev/null 2>&1 &&
    [ "$(cat "$work/seen")" = Fresh.kt ]
}

kotlin_run_leaves_no_generated_sources() { ! ls "$repo/kotlin/src/test/kotlin/app/media"/*.kt > /dev/null 2>&1; }

kotlin_run_returns_the_command_code() {
  PATH="$repo/bin:$PATH" bash "$repo/scripts/check_gen_consumers.sh" kotlin-run sh -c 'exit 7' > /dev/null 2>&1
  [ "$?" = 7 ] && kotlin_run_leaves_no_generated_sources
}

gradle_test_step_runs_through_kotlin_run() { grep -Eq '^kotlin\|gradle-test\|.*check_gen_consumers.sh kotlin-run ' "$work/list"; }

inv_check_13_gradle_test_compiles_only_freshly_generated_consumer_sources() {
  kotlin_run_repo
  expect_green_that "the sources the command sees are the freshly generated ones, the stale file is gone" kotlin_run_compiles_only_fresh_sources
  expect_green_that "no generated source is left behind after the run" kotlin_run_leaves_no_generated_sources
  kotlin_run_repo
  expect_green_that "the command code is returned and the sources are removed after a red run" kotlin_run_returns_the_command_code
  table_repo
  listed > "$work/list"
  expect_green_that "the gradle-test step generates its consumer sources itself" gradle_test_step_runs_through_kotlin_run
}

for test_name in $(declare -F | awk '{print $3}' | grep '^inv_check_'); do
  "$test_name"
  echo "$test_name: $([ "$failed" -eq 0 ] && echo ok || echo FAILED)"
done
[ "$failed" -eq 0 ] || exit 1
echo "check_invariants_selftest: ok"
