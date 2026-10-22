#!/usr/bin/env bash
# check_swift_test.sh [<simulator UDID> | <xcodebuild destination>]: the Swift package tests on an iOS Simulator.
# Without an argument the first booted iPhone simulator runs them, else the first available one.
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
exec xcodebuild test -workspace "$root/MediaLayer" -scheme MediaLayer-Package -destination "$destination" -derivedDataPath "$root/.orca-artifacts/DD.noindex"
