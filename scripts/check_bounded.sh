#!/usr/bin/env bash
# check_bounded.sh <seconds> <name> <command...>: runs the command under the step limit and kills its whole process tree
# past it. The check table runs every step through it.
set -uo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "$here/build_common.sh"
# shellcheck source=check_steps.sh
source "$here/check_steps.sh"
limit="$1"
name="$2"
shift 2
case "$name:$(basename -- "$1")" in gradle-step:* | *:gradle | *:gradlew)  refuse_gradle_on_darwin "step $name" || exit $? ;; esac
run_bounded "$limit" "$name" "$@"
