#!/usr/bin/env bash
# Self test: on Darwin the Gradle step and build_kotlin.sh refuse before any side effect, on Linux they reach their tools.
# Stubs stand in for uname, gradle, cargo and cargo-ndk, no real build runs. SCRIPTS_DIR names the scripts under test.
set -uo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
scripts_dir="${SCRIPTS_DIR:-$here}"
fail=0

# case_run <uname> <expect refuse|reach> <label> <script> <args...>
case_run() {
  local os="$1" expect="$2" label="$3" script="$4" tmp calls code=0 err
  shift 4
  tmp="$(mktemp -d)"
  mkdir -p "$tmp/scripts" "$tmp/bin" "$tmp/home" "$tmp/core" "$tmp/kotlin/src/main/kotlin/com/noma4i/medialayer"
  for f in "$scripts_dir"/*.sh; do sed '/opt\/homebrew\/opt\/rustup/d' "$f" > "$tmp/scripts/$(basename "$f")"; done
  : > "$tmp/core/Cargo.toml"
  : > "$tmp/kotlin/src/main/kotlin/com/noma4i/medialayer/medialayer.kt"
  printf '#!/bin/sh\n[ "$1" = -s ] && echo %s || echo %s\n' "$os" "$os" > "$tmp/bin/uname"
  printf '#!/bin/sh\necho 8\n' > "$tmp/bin/sysctl"
  for tool in gradle cargo cargo-ndk; do printf '#!/bin/sh\necho %s >> "%s/calls.log"\n' "$tool" "$tmp" > "$tmp/bin/$tool"; done
  chmod +x "$tmp"/bin/*
  err="$(cd "$tmp" && PATH="$tmp/bin:$PATH" HOME="$tmp/home" GRADLE="$tmp/bin/gradle" ANDROID_NDK_HOME="$tmp" bash "scripts/$script" "$@" 2>&1 > /dev/null)" || code=$?
  calls="$(cat "$tmp/calls.log" 2> /dev/null)"
  if [ "$expect" = refuse ]; then
    if [ "$code" -ne 0 ] && [ -z "$calls" ] && [ -e "$tmp/kotlin/src/main/kotlin/com/noma4i/medialayer/medialayer.kt" ] && [[ "$err" == *"gradle runs only on brick"* ]]; then
      echo "ok   $label (rc=$code, no tool called, binding kept)"
    else
      echo "FAIL $label: rc=$code calls=[$(echo "$calls" | tr '\n' ' ')] message=[$(echo "$err" | head -1)]"; fail=1
    fi
  elif [ -n "$calls" ]; then
    echo "ok   $label (reached: $(echo "$calls" | head -1))"
  else
    echo "FAIL $label: no tool reached, rc=$code"; fail=1
  fi
  rm -rf "$tmp"
}

for arg in --host --abis --release ""; do
  case_run Darwin refuse "darwin build_kotlin ${arg:-noargs}" build_kotlin.sh $arg
done
case_run Darwin refuse "darwin direct gradle-step" check_bounded.sh 5 gradle-step gradle -p kotlin testDebugUnitTest
case_run Darwin refuse "darwin direct gradle, any step name" check_bounded.sh 5 other-name gradle -p kotlin testDebugUnitTest
case_run Darwin refuse "darwin check kotlin lane" check_medialayer.sh --only kotlin
case_run Linux reach "linux build_kotlin --host" build_kotlin.sh --host
case_run Linux reach "linux direct gradle-step" check_bounded.sh 5 gradle-step gradle -p kotlin testDebugUnitTest
case_run Linux reach "linux direct gradle, any step name" check_bounded.sh 5 other-name gradle -p kotlin testDebugUnitTest
case_run Linux reach "linux check kotlin lane" check_medialayer.sh --only kotlin
exit "$fail"
