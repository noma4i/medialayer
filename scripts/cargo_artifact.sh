#!/usr/bin/env bash
# Sourced by the build scripts. reported_artifact <bin|kind> <name> reads `cargo build --message-format=json-render-diagnostics` from stdin
# and prints the path cargo reports for the artifact, wherever CARGO_TARGET_DIR puts it.
reported_artifact() {
  python3 -c 'import json, sys
kind, name = sys.argv[1:]
for line in sys.stdin:
    message = json.loads(line)
    if message.get("reason") == "compiler-artifact" and message["target"]["name"] == name and kind in message["target"]["kind"]:
        print(message["executable"] if kind == "bin" else next(f for f in message["filenames"] if not f.endswith((".rlib", ".rmeta"))))' "$@"
}
