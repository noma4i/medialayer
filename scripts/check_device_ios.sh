#!/usr/bin/env bash
# check_device_ios.sh <device UDID>: runs the MediaLayerProbe tests on a real iPhone (real camera and microphone) and saves the run under target/device-ios/<time>/.
# The team comes from probe/ios/Local.xcconfig (untracked, see Local.xcconfig.example) or from the environment DEVELOPMENT_TEAM and PROBE_BUNDLE_ID.
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
udid="${1:-}"
[ -n "$udid" ] || { echo "usage: check_device_ios.sh <device UDID>" >&2; exit 2; }
project="$root/probe/ios"
run="$root/target/device-ios/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$run"

extra=()
probe_run="$(basename "$run")"
[ -z "${DEVELOPMENT_TEAM:-}" ] || extra+=("DEVELOPMENT_TEAM=$DEVELOPMENT_TEAM")
[ -z "${PROBE_BUNDLE_ID:-}" ] || extra+=("PROBE_BUNDLE_ID=$PROBE_BUNDLE_ID")
# PROBE_ONLY: space-separated test names (MediaLayerProbeTests/ProbeTests/<test>) to run only those, for a single question on the shared phone.
for only in ${PROBE_ONLY:-}; do extra+=("-only-testing:$only"); done
if [ ! -f "$project/Local.xcconfig" ]; then
  [ -n "${DEVELOPMENT_TEAM:-}" ] || { echo "check_device_ios: give DEVELOPMENT_TEAM or create probe/ios/Local.xcconfig" >&2; exit 2; }
  : > "$project/Local.xcconfig"
fi

# The device slice of the local xcframework; a framework that already has it is kept.
if ! grep -q 'ios-arm64</string>' "$root/MediaLayer/MediaLayerFFI.xcframework/Info.plist" 2>/dev/null; then
  PATH="$(dirname "$(rustup which rustc)"):$PATH" bash "$root/scripts/build_ios.sh" --device
fi

(cd "$project" && xcodegen generate --quiet)
status=0
# TEST_RUNNER_ variables reach the test process as PROBE_RUN: one folder per run, never wiped by the tests.
TEST_RUNNER_PROBE_RUN="$probe_run" xcodebuild test -project "$project/MediaLayerProbe.xcodeproj" -scheme MediaLayerProbe \
  -destination "platform=iOS,id=$udid" -allowProvisioningUpdates \
  -derivedDataPath "$run/DerivedData" -resultBundlePath "$run/result.xcresult" ${extra[@]+"${extra[@]}"} > "$run/xcodebuild.log" 2>&1 || status=$?

# The files the tests wrote: the clips, the first frames and probe.log of the app container.
bundle="$(grep -m1 -E 'PRODUCT_BUNDLE_IDENTIFIER = ' "$run/xcodebuild.log" | sed 's/.*= //' || true)"
bundle="${bundle:-${PROBE_BUNDLE_ID:-$(sed -n 's/^PROBE_BUNDLE_ID *= *//p' "$project/Local.xcconfig")}}"
bundle="${bundle:-com.noma4i.medialayer.probe}"
mkdir -p "$run/files"
xcrun devicectl device copy from --device "$udid" --domain-type appDataContainer --domain-identifier "$bundle" \
  --source "Documents/probe/$probe_run" --destination "$run/files" > "$run/copy.log" 2>&1 || echo "check_device_ios: the files were not copied, see $run/copy.log" >&2

# The phone is shared: the probe app never stays alive after a run, checked, not assumed.
# The test host can come back a moment after xcodebuild returns, so the check repeats until the app has stayed gone for two looks in a row.
probe_pids() {
  xcrun devicectl device info processes --device "$udid" --json-output "$run/processes.json" > /dev/null 2>&1 || { echo "?"; return; }
  python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(" ".join(str(p["processIdentifier"]) for p in d.get("result",{}).get("runningProcesses",[]) if "MediaLayerProbe" in p.get("executable","")))' "$run/processes.json"
}
gone=0
for attempt in 1 2 3 4 5 6 7 8 9 10; do
  pids="$(probe_pids)"
  if [ -z "$pids" ]; then
    gone=$((gone + 1))
    [ "$gone" -ge 2 ] && break
  else
    gone=0
    for pid in $pids; do [ "$pid" = "?" ] || xcrun devicectl device process terminate --device "$udid" --pid "$pid" > /dev/null 2>&1 || true; done
  fi
  sleep 3
done
# Nothing can bring it back afterwards (an automation runner bound to it re-fronts it): the probe app leaves the phone.
xcrun devicectl device uninstall app --device "$udid" "$bundle" > "$run/uninstall.log" 2>&1 || echo "check_device_ios: the probe app was not uninstalled, see $run/uninstall.log" >&2
if [ "$gone" -ge 2 ]; then
  echo "check_device_ios: probe app terminated (gone on two checks 3 s apart) and uninstalled"
else
  echo "check_device_ios: the probe app is STILL RUNNING on $udid, terminate it by hand" >&2
  status=1
fi

grep -h '^PROBE ' "$run/xcodebuild.log" "$run"/files/probe.log "$run"/files/*/probe.log 2>/dev/null | sort -u || true
grep -E "Test Suite 'All tests'|Executed [0-9]+ tests|TEST (SUCCEEDED|FAILED)|error:" "$run/xcodebuild.log" | tail -n 12 || true
if [ "$status" -eq 0 ]; then echo "verdict: PASS ($run)"; else echo "verdict: FAIL ($run)"; fi
exit "$status"
