#!/usr/bin/env bash
# Traceability report: invariant (specs/invariants.md) -> tests named inv_<area>_<nn>_* in core/src, gen/src, swift/Tests and kotlin test sources.
# Report mode always exits 0. --strict exits 1 when a REQUIRED invariant has no test, a test names an unknown invariant or ids repeat.
# --list prints every invariant with the names of its tests.
set -euo pipefail

mode=report
case "${1:-}" in
  --strict) mode=strict ;;
  --list) mode=list ;;
  "") ;;
  *) echo "usage: check_invariants.sh [--strict|--list]" >&2; exit 2 ;;
esac

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
spec="$root/specs/invariants.md"

if [ ! -f "$spec" ]; then
  echo "check_invariants: $spec is missing (specs/ is not tracked), nothing to report"
  [ "$mode" = strict ] && exit 1
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

awk -F'|' '/^\| INV-[A-Z]+-[0-9]+ \| (REQUIRED|UNBUILT|OPEN) \|/ {
  id = $2; status = $3
  gsub(/ /, "", id); gsub(/ /, "", status)
  print id, status
}' "$spec" > "$work/registry.txt"

if [ ! -s "$work/registry.txt" ]; then
  echo "check_invariants: no invariant rows found in $spec" >&2
  exit 1
fi

dups="$(awk '{print $1}' "$work/registry.txt" | sort | uniq -d)"

{
  grep -rhoE --include='*.rs' 'fn inv_[a-z0-9_]+' "$root/core/src" "$root/gen/src" 2>/dev/null || true
  grep -rhoE --include='*.swift' 'func inv_[a-z0-9_]+' "$root/swift/Tests" 2>/dev/null || true
  grep -rhoE --include='*.kt' 'fun inv_[a-z0-9_]+' "$root/kotlin/src/test" "$root/kotlin/ui/src/test" 2>/dev/null || true
} | sed -E 's/^(fn|func|fun) //' | sort -u > "$work/tests.txt"

if [ "$mode" = list ]; then
  while read -r id status; do
    prefix="$(printf '%s' "$id" | tr 'A-Z-' 'a-z_')_"
    echo "$id $status"
    grep -F "$prefix" "$work/tests.txt" | sed 's/^/    /' || true
  done < "$work/registry.txt"
  exit 0
fi

awk -v tests="$work/tests.txt" '
BEGIN {
  while ((getline line < tests) > 0) names[++count] = line
  printf "%-24s %-9s %-6s %s\n", "invariant", "status", "tests", "covered"
}
{
  id = $1; status = $2
  prefix = tolower(id); gsub(/-/, "_", prefix); prefix = prefix "_"
  found = 0
  for (i = 1; i <= count; i++) if (index(names[i], prefix) > 0) found++
  covered = (found > 0) ? "yes" : "no"
  if (status != "REQUIRED" && found == 0) covered = "-"
  printf "%-24s %-9s %-6d %s\n", id, status, found, covered
  total[status]++
  if (status == "REQUIRED") { if (found > 0) done++; else missing++ }
  if (status != "REQUIRED" && found > 0) early++
}
END {
  printf "\nREQUIRED %d: with tests %d, without tests %d\n", total["REQUIRED"], done, missing
  printf "UNBUILT %d, OPEN %d, tests ahead of status %d\n", total["UNBUILT"], total["OPEN"], early
  print missing + 0 > "/dev/stderr"
}' "$work/registry.txt" > "$work/table.txt" 2> "$work/missing.txt"

cat "$work/table.txt"

known="$(awk '{print tolower($1)}' "$work/registry.txt" | tr '-' '_')"
orphans=""
while IFS= read -r name; do
  [ -z "$name" ] && continue
  for part in $(printf '%s\n' "$name" | grep -oE 'inv_[a-z]+_[0-9]{2}_' || true); do
    id="${part%_}"
    if ! printf '%s\n' "$known" | grep -qx "$id"; then
      orphans="$orphans$name (unknown invariant $id)"$'\n'
    fi
  done
done < "$work/tests.txt"

if [ -n "$dups" ]; then
  echo "duplicate invariant ids in registry: $(echo $dups)"
fi
if [ -n "$orphans" ]; then
  echo "tests naming an unknown invariant:"
  printf '%s' "$orphans"
fi

missing="$(tr -d '[:space:]' < "$work/missing.txt")"
if [ "$mode" = strict ]; then
  if [ "${missing:-0}" != "0" ] || [ -n "$orphans" ] || [ -n "$dups" ]; then
    echo "check_invariants --strict: failed (REQUIRED without tests: ${missing:-0})" >&2
    exit 1
  fi
  echo "check_invariants --strict: ok"
else
  echo "check_invariants: report only, REQUIRED without tests: ${missing:-0} (use --strict to fail)"
fi
