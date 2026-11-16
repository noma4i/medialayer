#!/usr/bin/env bash
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
case "${1:-}" in
  swift)
    git -C "$root" ls-files -z 'MediaLayer/Sources/**/*.swift' | python3 -c 'import pathlib,sys; paths=sys.stdin.buffer.read().split(b"\0"); root=pathlib.Path(sys.argv[1]); missing=[p.decode() for p in paths if p and not (root / p.decode()).is_file()]; sys.exit("missing tracked Swift sources: " + ", ".join(missing) if missing else 0)' "$root"
    built="$(python3 -B "$root/scripts/swift_products.py")"
    products="$(sed -n 1p <<< "$built")"
    target="$(sed -n 2p <<< "$built")"
    mkdir -p "$root/target/api-schema"
    output="$(mktemp -d "$root/target/api-schema/swift.XXXXXX")"
    sdk="$(xcrun --sdk iphonesimulator --show-sdk-path)"
    for module in MediaLayer MediaLayerUI MediaLayerTesting; do
      xcrun swift-symbolgraph-extract -module-name "$module" -target "$target" -sdk "$sdk" -I "$products" -I "$root/target/ios-build/headers/medialayerFFI" -F "$products" -minimum-access-level public -output-dir "$output"
    done
    printf '%s\n' "$output" > "$root/target/api-schema/swift-path"
    ;;
  kotlin)
    git -C "$root" ls-files -z 'kotlin/src/main/**/*.kt' 'kotlin/ui/src/main/**/*.kt' 'kotlin/testing/src/main/**/*.kt' | python3 -c 'import pathlib,sys; paths=sys.stdin.buffer.read().split(b"\0"); root=pathlib.Path(sys.argv[1]); missing=[p.decode() for p in paths if p and not (root / p.decode()).is_file()]; sys.exit("missing tracked Kotlin sources: " + ", ".join(missing) if missing else 0)' "$root"
    "${GRADLE:-gradle}" -p "$root/kotlin" --no-watch-fs :dumpPublicApi :ui:dumpPublicApi :testing:dumpPublicApi
    ;;
  *) echo 'usage: api_schema.sh <swift|kotlin>' >&2; exit 2 ;;
esac
