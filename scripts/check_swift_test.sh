#!/usr/bin/env bash
# check_swift_test.sh [<simulator UDID> | <xcodebuild destination> [<extra xcodebuild argument>...]]: the Swift package tests on an iOS Simulator.
# Without an argument the first booted iPhone simulator runs them, else the first available one. Extra arguments such as -only-testing:<target>/<suite> or -derivedDataPath <dir> go to xcodebuild.
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
destination="${1:-}"
case "$destination" in
  *=*) ;;
  "")
    uuid='[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}'
    devices="$(xcrun simctl list devices available iPhone)"
    booted="$(grep -F '(Booted)' <<< "$devices" || true)"
    udid="$(grep -oE "$uuid" <<< "$booted$devices" | sed -n 1p)"
    [ -n "$udid" ] || { echo "check_swift_test: no iPhone simulator is available" >&2; exit 1; }
    destination="platform=iOS Simulator,id=$udid"
    ;;
  *) destination="platform=iOS Simulator,id=$destination" ;;
esac
if [ "$#" -gt 0 ]; then shift; fi
derived=(-derivedDataPath "$root/.orca-artifacts/DD.noindex")
for argument in "$@"; do
  if [ "$argument" = -derivedDataPath ]; then derived=(); fi
done
exec xcodebuild test -workspace "$root/MediaLayer" -scheme MediaLayer-Package -destination "$destination" ${derived[@]+"${derived[@]}"} "$@"
