#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
fail() { printf 'release_android: %s\n' "$*" >&2; exit 1; }
[ "$#" -eq 2 ] || fail 'usage: release_android.sh <version> <out-dir>'
version="$1"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9.-]+)?$ ]] || fail 'version must be x.y.z with an optional qualifier'
out_dir="$(mkdir -p -- "$2" && cd -- "$2" && pwd -P)"
if [ -n "$(find "$out_dir" -mindepth 1 -maxdepth 1 -print -quit)" ]; then
  fail "output directory is not empty: $out_dir"
fi

bash "$root/scripts/build_kotlin.sh"
gradle="${GRADLE:-$(ls -d "${HOME:?}"/.gradle/wrapper/dists/gradle-9.*-bin/*/gradle-9.*/bin/gradle 2>/dev/null | sort -V | tail -1)}"
[ -x "$gradle" ] || fail 'Gradle 9 not found: set GRADLE'
staging="$out_dir/maven"
mkdir -p "$staging"
"$gradle" -p "$root/kotlin" --no-watch-fs -PreleaseVersion="$version" -PstagingRepo="$staging" :publishReleasePublicationToStagingRepository :ui:publishReleasePublicationToStagingRepository

for artifact in medialayer medialayer-ui; do
  artifact_dir="$staging/com/noma4i/$artifact/$version"
  for extension in aar pom module; do
    [ -s "$artifact_dir/$artifact-$version.$extension" ] || fail "missing $artifact $extension artifact"
  done
done

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/core-aar" "$tmp/core-classes" "$tmp/ui-aar"
(cd "$tmp/core-aar" && unzip -q "$staging/com/noma4i/medialayer/$version/medialayer-$version.aar")
for abi in arm64-v8a armeabi-v7a x86_64 x86; do
  [ -s "$tmp/core-aar/jni/$abi/libmedialayer.so" ] || fail "medialayer AAR missing ABI $abi"
done
[ -s "$tmp/core-aar/classes.jar" ] || fail 'medialayer AAR missing classes.jar'
(cd "$tmp/core-classes" && unzip -q "$tmp/core-aar/classes.jar")
[ -n "$(find "$tmp/core-classes/com/noma4i/medialayer" -name '*.class' -print -quit 2>/dev/null)" ] || fail 'medialayer AAR missing Kotlin classes'
(cd "$tmp/ui-aar" && unzip -q "$staging/com/noma4i/medialayer-ui/$version/medialayer-ui-$version.aar")
[ -s "$tmp/ui-aar/classes.jar" ] || fail 'medialayer-ui AAR missing classes.jar'
grep -Fq '<artifactId>medialayer</artifactId>' "$staging/com/noma4i/medialayer-ui/$version/medialayer-ui-$version.pom" || fail 'medialayer-ui POM missing medialayer dependency'

archive="$out_dir/medialayer-android.zip"
(cd "$root/kotlin" && zip -q -r "$archive" \
  src/main/jniLibs/arm64-v8a/libmedialayer.so \
  src/main/jniLibs/armeabi-v7a/libmedialayer.so \
  src/main/jniLibs/x86_64/libmedialayer.so \
  src/main/jniLibs/x86/libmedialayer.so \
  src/main/kotlin/com/noma4i/medialayer/medialayer.kt)
archive_entries="$(unzip -Z1 "$archive")"
for entry in \
  src/main/jniLibs/arm64-v8a/libmedialayer.so \
  src/main/jniLibs/armeabi-v7a/libmedialayer.so \
  src/main/jniLibs/x86_64/libmedialayer.so \
  src/main/jniLibs/x86/libmedialayer.so \
  src/main/kotlin/com/noma4i/medialayer/medialayer.kt; do
  grep -Fxq "$entry" <<< "$archive_entries" || fail "Android archive missing $entry"
done
printf 'Android release artifacts: %s\n' "$out_dir"
