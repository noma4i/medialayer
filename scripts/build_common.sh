#!/usr/bin/env bash
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(getconf _NPROCESSORS_ONLN)}" RUST_TEST_THREADS="${RUST_TEST_THREADS:-$(getconf _NPROCESSORS_ONLN)}"
reported_artifact() {
  python3 -c 'import json, sys
kind, name = sys.argv[1:]
suffixes = {"staticlib": (".a",), "cdylib": (".so", ".dylib", ".dll")}
result = None
error = None
for line in sys.stdin:
    message = json.loads(line)
    if message.get("reason") == "compiler-artifact" and message["target"]["name"] == name and kind in message["target"]["kind"]:
        if kind == "bin":
            result = message["executable"]
        else:
            found = [f for f in message["filenames"] if f.endswith(suffixes[kind])]
            if not found:
                error = "reported_artifact: no " + kind + " file for " + name + " in " + str(message["filenames"])
            else:
                result = found[0]
if error:
    sys.exit(error)
if result:
    print(result)
    sys.exit(0)
sys.exit("reported_artifact: no " + kind + " artifact " + name)' "$@"
}
