#!/usr/bin/env bash
# check_swift_test.sh: the Swift package tests on an iOS Simulator. Where sim-run exists (SIM_RUN or ~/.claude/bin/sim-run) it picks the device;
# elsewhere the first booted iPhone simulator runs them, else the first available one.
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
sim_run="${SIM_RUN:-$HOME/.claude/bin/sim-run}"
test_command=(xcodebuild test -workspace "$root/MediaLayer" -scheme MediaLayer-Package -destination "platform=iOS Simulator,id=@UDID@" -derivedDataPath "$root/.orca-artifacts/DD.noindex")

if [ -x "$sim_run" ]; then
  exec "$sim_run" swift-test -- "${test_command[@]}"
fi

uuid='[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}'
devices="$(xcrun simctl list devices available iPhone)"
booted="$(grep -F '(Booted)' <<< "$devices" || true)"
udid="$(grep -oE "$uuid" <<< "$booted$devices" | sed -n 1p)"
[ -n "$udid" ] || { echo "check_swift_test: no iPhone simulator is available" >&2; exit 1; }
exec "${test_command[@]/@UDID@/$udid}"
