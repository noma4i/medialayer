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
| INV-T-02 | UNBUILT | f | s | c | I | break it | A; reason: not built yet |
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

inv_check_03_unbuilt_needs_a_reason_and_required_needs_a_mutation() {
  fixture
  sed -i '' 's/; reason: not built yet//' "$repo/specs/invariants.md"
  expect_red "unbuilt without a reason"
  fixture
  sed -i '' '/INV-T-02/d; s/| break it |/|  |/' "$repo/specs/invariants.md"
  expect_red "required without a mutation"
}

inv_check_04_a_missing_registry_is_red() {
  fixture
  rm "$repo/specs/invariants.md"
  expect_red "registry removed"
}

inv_check_05_a_step_past_its_limit_leaves_no_process() {
  local code=0
  run_bounded 2 selftest bash -c 'set -m; sleep 601.7 & wait' 2> /dev/null || code=$?
  sleep 1
  if [ "$code" -ne 124 ] || pgrep -f 'sleep 601.7' > /dev/null; then
    echo "FAIL expect_red: step code $code, leftover: $(pgrep -fl 'sleep 601.7' || true)"
    pkill -f 'sleep 601.7' || true
    failed=1
  fi
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

engine() {
  root="$work/engine"
  mkdir -p "$root/target"
  rm -f "$root"/ran-*
  changed_files() { echo "$CHANGED"; }
  steps=(
    "cheap|c1|20|*|$c1"
    "core|k1|20|core/|touch ran-k1 && $k1"
    "swift|s1|20|swift/|touch ran-s1 && $s1"
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
  c1=true; k1=true; s1=true; CHANGED="core/a.rs"
  expect_green_engine "all steps pass in full"
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

for test_name in $(declare -F | awk '{print $3}' | grep '^inv_check_'); do
  "$test_name"
  echo "$test_name: $([ "$failed" -eq 0 ] && echo ok || echo FAILED)"
done
[ "$failed" -eq 0 ] || exit 1
echo "check_invariants_selftest: ok"
