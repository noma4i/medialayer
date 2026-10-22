#!/usr/bin/env bash
# Sourced by the build scripts. reported_artifact <bin|kind> <name> reads `cargo build --message-format=json-render-diagnostics` from stdin
# and prints the path cargo reports for the artifact, wherever CARGO_TARGET_DIR puts it.
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
