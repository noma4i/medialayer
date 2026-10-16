#!/usr/bin/env bash
# Sourced by build_kotlin.sh, check_bounded.sh and check_steps.sh. Gradle and the Kotlin build run only on brick.
refuse_gradle_on_darwin() {
  [ "$(uname -s)" = Darwin ] || return 0
  echo "gradle runs only on brick: $1 refused on Darwin, nothing was started" >&2
  return 3
}
