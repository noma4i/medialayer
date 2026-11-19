#!/usr/bin/env bash
# Runs the phone probe on a real Android device and prints its PROBE lines and the verdict.
# Environment:
#   ANDROID_SERIAL  device serial (required when more than one device is attached)
#   ADB             adb binary (default: adb)
#   APK             a ready test APK; or
#   BUILD_SSH + BUILD_DIR + GRADLE   build it there (arm64 jniLibs, :ui:assembleDebugAndroidTest) and copy it back
#   BUILD_ENV       optional shell snippet sourced on the build host before the build
#   CLASSES         comma list of instrumented classes (default: the phone probe, real camera and microphone only)
#   TEST_PACKAGE    package of the test APK (default: com.noma4i.medialayer.ui.test)
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
adb="${ADB:-adb}"
classes="${CLASSES:-com.noma4i.medialayer.ui.CameraPhoneProbeTest}"
package="${TEST_PACKAGE:-com.noma4i.medialayer.ui.test}"
fail() { printf 'check_device_android: %s\n' "$*" >&2; exit 1; }

out="$root/target/device-android/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$out"

if [ -z "${APK:-}" ]; then
  [ -n "${BUILD_SSH:-}" ] && [ -n "${BUILD_DIR:-}" ] || fail 'set APK, or BUILD_SSH and BUILD_DIR'
  gradle="${GRADLE:-gradle}"
  # shellcheck disable=SC2029
  ssh -n -o BatchMode=yes "$BUILD_SSH" "set -e; ${BUILD_ENV:-:}; cd '$BUILD_DIR'; bash scripts/build_kotlin.sh --release; timeout 1800 '$gradle' --offline -p kotlin :ui:assembleDebugAndroidTest" > "$out/build.log" 2>&1 || { tail -n 40 "$out/build.log" >&2; fail "build failed, log $out/build.log"; }
  remote="$(ssh -n -o BatchMode=yes "$BUILD_SSH" "ls -t '$BUILD_DIR'/kotlin/ui/build/outputs/apk/androidTest/debug/*.apk | head -n 1")"
  [ -n "$remote" ] || fail 'no test APK produced'
  scp -q -o BatchMode=yes "$BUILD_SSH:$remote" "$out/test.apk"
  APK="$out/test.apk"
fi
[ -s "$APK" ] || fail "APK not found: $APK"

"$adb" get-state >/dev/null 2>&1 || fail 'no device on adb'
"$adb" install -r -t "$APK" > "$out/install.log" 2>&1 || { cat "$out/install.log" >&2; fail 'install failed'; }
"$adb" logcat -c
"$adb" shell am instrument -w -e class "$classes" "$package/androidx.test.runner.AndroidJUnitRunner" < /dev/null > "$out/instrument.txt" 2>&1 || true
"$adb" logcat -d -s MediaLayerTrace MediaLayerProbe > "$out/logcat.txt" 2>&1 || true

grep -h 'PROBE ' "$out/logcat.txt" "$out/instrument.txt" | sed 's/^.*PROBE /PROBE /' | sort -u || true
ran="$(sed -n 's/^OK (\([0-9]*\) tests\{0,1\})/\1/p' "$out/instrument.txt" | head -n 1)"
if grep -q '^FAILURES!!!' "$out/instrument.txt"; then
  verdict=FAIL
elif [ "${ran:-0}" -gt 0 ] 2>/dev/null; then
  verdict=PASS
else
  verdict=FAIL
  echo 'no test ran' >&2
fi
echo "verdict=$verdict run=$out"
[ "$verdict" = PASS ]
