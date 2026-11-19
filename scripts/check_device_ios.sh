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
[ -z "${DEVELOPMENT_TEAM:-}" ] || extra+=("DEVELOPMENT_TEAM=$DEVELOPMENT_TEAM")
[ -z "${PROBE_BUNDLE_ID:-}" ] || extra+=("PROBE_BUNDLE_ID=$PROBE_BUNDLE_ID")
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
xcodebuild test -project "$project/MediaLayerProbe.xcodeproj" -scheme MediaLayerProbe \
  -destination "platform=iOS,id=$udid" -allowProvisioningUpdates \
  -derivedDataPath "$run/DerivedData" -resultBundlePath "$run/result.xcresult" ${extra[@]+"${extra[@]}"} > "$run/xcodebuild.log" 2>&1 || status=$?

# The files the tests wrote: the clips, the first frames and probe.log of the app container.
bundle="$(grep -m1 -E 'PRODUCT_BUNDLE_IDENTIFIER = ' "$run/xcodebuild.log" | sed 's/.*= //' || true)"
bundle="${bundle:-${PROBE_BUNDLE_ID:-$(sed -n 's/^PROBE_BUNDLE_ID *= *//p' "$project/Local.xcconfig")}}"
bundle="${bundle:-com.noma4i.medialayer.probe}"
mkdir -p "$run/files"
xcrun devicectl device copy from --device "$udid" --domain-type appDataContainer --domain-identifier "$bundle" \
  --source Documents/probe --destination "$run/files" > "$run/copy.log" 2>&1 || echo "check_device_ios: the files were not copied, see $run/copy.log" >&2

grep -h '^PROBE ' "$run/xcodebuild.log" "$run"/files/probe.log "$run"/files/*/probe.log 2>/dev/null | sort -u || true
grep -E "Test Suite 'All tests'|Executed [0-9]+ tests|TEST (SUCCEEDED|FAILED)|error:" "$run/xcodebuild.log" | tail -n 12 || true
if [ "$status" -eq 0 ]; then echo "verdict: PASS ($run)"; else echo "verdict: FAIL ($run)"; fi
exit "$status"
