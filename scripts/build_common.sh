#!/usr/bin/env bash
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(getconf _NPROCESSORS_ONLN)}" RUST_TEST_THREADS="${RUST_TEST_THREADS:-$(getconf _NPROCESSORS_ONLN)}"
reported_artifact() {
  python3 -c 'import json, sys
kind, name = sys.argv[1:]
suffixes = {"staticlib": (".a",), "cdylib": (".so", ".dylib", ".dll")}
for line in sys.stdin:
    message = json.loads(line)
    if message.get("reason") == "compiler-artifact" and message["target"]["name"] == name and kind in message["target"]["kind"]:
        if kind == "bin":
            print(message["executable"])
            sys.exit(0)
        found = [f for f in message["filenames"] if f.endswith(suffixes[kind])]
        if not found:
            sys.exit("reported_artifact: no " + kind + " file for " + name + " in " + str(message["filenames"]))
        print(found[0])
        sys.exit(0)
sys.exit("reported_artifact: no " + kind + " artifact " + name)' "$@"
}
