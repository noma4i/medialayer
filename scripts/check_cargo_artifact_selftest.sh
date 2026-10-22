#!/usr/bin/env bash
# Self test: reported_artifact picks the file by artifact kind, not by position, on recorded cargo output.
set -uo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=cargo_artifact.sh
source "${SCRIPTS_DIR:-$here}/cargo_artifact.sh"
fail=0

# artifact_line <kind> <filenames...> prints one compiler-artifact message
artifact_line() {
  local kind="$1" files
  shift
  files="$(printf '"/t/%s",' "$@")"
  printf '{"reason":"compiler-artifact","target":{"name":"medialayer","kind":["lib","cdylib","staticlib"]},"filenames":[%s],"executable":null,"kind":"%s"}\n' "${files%,}" "$kind"
}

# case_run <label> <kind> <expected path or FAIL> <filenames...>
case_run() {
  local label="$1" kind="$2" expect="$3" out code=0
  shift 3
  out="$(artifact_line "$kind" "$@" | reported_artifact "$kind" medialayer 2> /dev/null)" || code=$?
  if [ "$expect" = FAIL ]; then
    if [ "$code" -ne 0 ] && [ -z "$out" ]; then echo "ok   $label (rc=$code)"; else echo "FAIL $label: rc=$code out=[$out]"; fail=1; fi
  elif [ "$code" -eq 0 ] && [ "$out" = "/t/$expect" ]; then
    echo "ok   $label"
  else
    echo "FAIL $label: rc=$code out=[$out] want=[/t/$expect]"; fail=1
  fi
}

case_run "staticlib, dylib listed first" staticlib libmedialayer.a libmedialayer.rlib libmedialayer.dylib libmedialayer.a
case_run "staticlib, .a listed first" staticlib libmedialayer.a libmedialayer.a libmedialayer.rlib libmedialayer.dylib
case_run "cdylib, dylib" cdylib libmedialayer.dylib libmedialayer.rlib libmedialayer.dylib libmedialayer.a
case_run "cdylib, .a listed first" cdylib libmedialayer.so libmedialayer.a libmedialayer.rlib libmedialayer.so
case_run "staticlib, no .a" staticlib FAIL libmedialayer.rlib libmedialayer.dylib
case_run "cdylib, no shared library" cdylib FAIL libmedialayer.rlib libmedialayer.a

exit "$fail"
