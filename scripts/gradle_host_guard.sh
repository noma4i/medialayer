#!/usr/bin/env bash
# Sourced by check_bounded.sh and check_steps.sh. Gradle runs only on brick.
refuse_gradle_on_darwin() {
  [ "$(uname -s)" = Darwin ] || return 0
  echo "gradle runs only on brick: $1 refused on Darwin, nothing was started" >&2
  return 3
}
