#!/usr/bin/env bash
# Self test of the checks themselves: INV-CHECK-01..07. Each function builds a fixture repository, breaks it in
# one way and expects the matching check to go red. Runs in seconds, takes no build slot.
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
  run_bounded 2 selftest bash -c 'set -m; sleep 601.7 & wait' 2> /dev/null || code=$?
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

for test_name in $(declare -F | awk '{print $3}' | grep '^inv_check_'); do
  "$test_name"
  echo "$test_name: $([ "$failed" -eq 0 ] && echo ok || echo FAILED)"
done
[ "$failed" -eq 0 ] || exit 1
echo "check_invariants_selftest: ok"
