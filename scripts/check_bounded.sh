#!/usr/bin/env bash
# check_bounded.sh <seconds> <name> <command...>: runs the command under the step limit and kills its whole process tree
# past it. The check table puts this inside the build slot, so the limit starts after the slot is taken.
set -uo pipefail
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(sysctl -n hw.ncpu)}" RUST_TEST_THREADS="${RUST_TEST_THREADS:-$(sysctl -n hw.ncpu)}"
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=check_steps.sh
source "$here/check_steps.sh"
limit="$1"
name="$2"
shift 2
run_bounded "$limit" "$name" "$@"
