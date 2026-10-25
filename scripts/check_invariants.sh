#!/usr/bin/env bash
# Strict traceability: invariant (specs/invariants.md) -> test inv_<area>_<nn>_* with an assertion in its body.
# Exit 1 when a REQUIRED row has no such test, an UNBUILT row has no real `reason:` in its last cell (at least 25 characters, not the restated status or the docs template), a REQUIRED row has no
# mutation text, a test, doc or spec names an invariant that has no row, ids repeat, or the registry is missing. There is no mode that exits 0 on a gap.
# --list prints every invariant with the names of its tests.
# Env: SPEC (registry file), TEST_DIRS (space separated, relative to the repository root) for the self test.
set -euo pipefail

mode=check
case "${1:-}" in
  --list) mode=list ;;
  "") ;;
  *) echo "usage: check_invariants.sh [--list]" >&2; exit 2 ;;
esac

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
spec="${SPEC:-$root/specs/invariants.md}"
if [ ! -f "$spec" ] && [ -z "${SPEC:-}" ]; then
  common="$(git -C "$root" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
  [ -n "$common" ] && spec="$(dirname -- "$common")/specs/invariants.md"
fi
if [ ! -f "$spec" ]; then
  echo "check_invariants: registry $spec is missing" >&2
  exit 1
fi

test_dirs="${TEST_DIRS:-core/src gen/src gen/fixtures swift/Tests MediaLayer/Tests kotlin/src/test kotlin/ui/src/test scripts}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

awk -F'|' '/^\| INV-[A-Z]+-[0-9]+ \| (REQUIRED|UNBUILT|OPEN) \|/ {
  id = $2; status = $3; mutation = $8
  gsub(/ /, "", id); gsub(/ /, "", status)
  why = $(NF - 1); sub(/^.*reason: */, "", why); sub(/ +$/, "", why)
  reason = ($(NF - 1) ~ /reason:/ && length(why) >= 25 && why !~ /no test named|^promised by the docs, no implementation$|^(todo|tbd|later|none|unbuilt)/) ? "reason" : "-"
  gsub(/[ `-]/, "", mutation)
  has = (length(mutation) > 0) ? "mutation" : "-"
  ran = (mutation ~ /ran:[0-9a-f]/) ? "ran" : "-"
  print id "\t" status "\t" reason "\t" has "\t" ran
}' "$spec" > "$work/registry.txt"

if [ ! -s "$work/registry.txt" ]; then
  echo "check_invariants: no invariant rows found in $spec" >&2
  exit 1
fi

dups="$(cut -f1 "$work/registry.txt" | sort | uniq -d | tr '\n' ' ')"

files=()
for dir in $test_dirs; do
  [ -d "$root/$dir" ] && while IFS= read -r file; do files+=("$file"); done < <(find "$root/$dir" \( -name '*.rs' -o -name '*.swift' -o -name '*.kt' -o -name '*.sh' \) -not -path '*/target*' -not -path '*/build/*' -not -path '*/.build/*')
done

# name, 1 when the body holds an assertion, else 0
if [ "${#files[@]}" -gt 0 ]; then
  awk '
  # clean(): drops comments and string literals so that names, braces and assertions are read from code only
  function clean(line, sh,    out, i, n, c, pair, triple) {
    out = ""; n = length(line); i = 1
    while (i <= n) {
      c = substr(line, i, 1); pair = substr(line, i, 2); triple = substr(line, i, 3)
      if (block) { if (pair == "*/") { block = 0; i += 2 } else i++; continue }
      if (multiline) { if (triple == "\"\"\"") { multiline = 0; i += 3 } else i++; continue }
      if (!sh && pair == "/*") { block = 1; i += 2; continue }
      if (!sh && pair == "//") break
      if (sh && c == "#" && (i == 1 || substr(line, i - 1, 1) ~ /[ \t]/)) break
      if (!sh && triple == "\"\"\"") { multiline = 1; i += 3; out = out "\"\""; continue }
      if (c == "\"") {
        i++
        while (i <= n) { c = substr(line, i, 1); if (c == "\\") { i += 2; continue } if (c == "\"") break; i++ }
        i++; out = out "\"\""; continue
      }
      out = out c; i++
    }
    return out
  }
  FNR == 1 { infn = 0; block = 0; multiline = 0; pending_ignore = 0 }
  {
    sh = (FILENAME ~ /\.sh$/)
    code = clean($0, sh)
    if (!infn) {
      name = ""
      if (!sh && match(code, /(fn|func|fun) +inv_[a-z0-9_]+/)) { name = substr(code, RSTART, RLENGTH); sub(/^(fn|func|fun) +/, "", name) }
      else if (sh && match(code, /^inv_[a-z0-9_]+\(\)/)) { name = substr(code, RSTART, RLENGTH); gsub(/[ ()]/, "", name) }
      if (name == "") {
        if (code ~ /#\[ignore|@Ignore|@Disabled|\.disabled/) pending_ignore = 1
        else if (code ~ /[^ \t]/ && code !~ /^[ \t]*(#\[|@)/) pending_ignore = 0
        next
      }
      infn = 1; depth = 0; started = 0; asserted = 0
      ignored = pending_ignore || (code ~ /#\[ignore|@Ignore|@Disabled|\.disabled/)
      pending_ignore = 0
    }
    opens = gsub(/\{/, "{", code); closes = gsub(/\}/, "}", code)
    if (opens > 0) started = 1
    depth += opens - closes
    if (code ~ /(^|[^A-Za-z0-9_])(assert[A-Za-z_]*!?\(|debug_assert[a-z_]*!\(|#expect\(|#require\(|XCTAssert[A-Za-z]*\(|XCTFail\(|Issue\.record\(|panic!\(|fail\(|wait_until[A-Za-z_]*\(|eventually\()/ || code ~ /(^|[^A-Za-z0-9_.])until\(/ || \
        (sh && code ~ /(^|[^A-Za-z0-9_])expect_(red|green)[a-z_]*([^A-Za-z0-9_]|$)/)) asserted = 1
    if (started && depth <= 0) { print name "\t" (ignored ? 0 : asserted); infn = 0 }
  }' "${files[@]}" | sort -u > "$work/tests.txt"
else
  : > "$work/tests.txt"
fi

if [ "$mode" = list ]; then
  while IFS=$'\t' read -r id status _; do
    prefix="$(printf '%s' "$id" | tr 'A-Z-' 'a-z_')_"
    echo "$id $status"
    awk -F'\t' -v p="$prefix" 'index($1, p) > 0 { print "    " $1 ($2 == 0 ? "  (no assertion)" : "") }' "$work/tests.txt"
  done < "$work/registry.txt"
  exit 0
fi

awk -F'\t' -v tests="$work/tests.txt" '
BEGIN {
  while ((getline line < tests) > 0) { split(line, p, "\t"); names[++count] = p[1]; asserts[count] = p[2] }
  printf "%-24s %-9s %-6s %-9s %s\n", "invariant", "status", "tests", "asserting", "verdict"
}
{
  id = $1; status = $2; reason = $3; mutation = $4; ran = $5
  prefix = tolower(id); gsub(/-/, "_", prefix); prefix = prefix "_"
  found = 0; good = 0
  for (i = 1; i <= count; i++) if (index(names[i], prefix) > 0) { found++; if (asserts[i] == 1) good++ }
  verdict = "-"
  total[status]++
  if (status == "REQUIRED") {
    if (good > 0) { verdict = "covered"; done++ }
    else if (found > 0) { verdict = "NO ASSERTION"; noassert++ }
    else { verdict = "NO TEST"; notest++ }
    if (mutation == "-") { verdict = verdict " NO MUTATION"; nomutation++ }
    if (ran == "ran") ranrows++
  } else if (status == "UNBUILT") {
    if (reason == "-") { verdict = "NO REASON"; noreason++ }
    else if (good > 0) { verdict = "test ahead of status"; ahead++ }
    else verdict = "unbuilt"
  }
  printf "%-24s %-9s %-6d %-9d %s\n", id, status, found, good, verdict
}
END {
  printf "\nREQUIRED %d: covered %d, no test %d, test without assertion %d, no mutation text %d, mutation run recorded %d\n", total["REQUIRED"], done, notest, noassert, nomutation, ranrows
  printf "UNBUILT %d (without reason %d, test ahead of status %d), OPEN %d\n", total["UNBUILT"], noreason, ahead, total["OPEN"]
  print notest + noassert + nomutation + noreason > "/dev/stderr"
}' "$work/registry.txt" > "$work/table.txt" 2> "$work/bad.txt"

cat "$work/table.txt"

doc_paths="${DOC_PATHS:-docs README.md specs}"
mentioned=""
for entry in $doc_paths; do
  [ -e "$root/$entry" ] && mentioned="$mentioned$(grep -rhoE 'INV-[A-Z]+-[0-9]+' "$root/$entry" 2> /dev/null | sort -u | tr '\n' ' ' || true)"
done
mentioned="$mentioned$(grep -hoE 'INV-[A-Z]+-[0-9]+' "$spec" | sort -u | tr '\n' ' ' || true)"
missing_rows=""
cut -f1 "$work/registry.txt" > "$work/ids.txt"
for id in $(printf '%s\n' $mentioned | sort -u); do
  grep -qx "$id" "$work/ids.txt" || missing_rows="$missing_rows$id "
done

awk -F'\t' '{print tolower($1)}' "$work/registry.txt" | tr '-' '_' > "$work/known.txt"
orphans=""
while IFS=$'\t' read -r name _; do
  [ -z "$name" ] && continue
  for part in $(printf '%s\n' "$name" | grep -oE 'inv_[a-z]+_[0-9]{2}_' || true); do
    id="${part%_}"
    grep -qx "$id" "$work/known.txt" || orphans="$orphans$name (unknown invariant $id)"$'\n'
  done
done < "$work/tests.txt"

[ -n "$dups" ] && echo "duplicate invariant ids in registry: $dups"
[ -n "$missing_rows" ] && echo "named by docs, specs or the registry but without a row: $missing_rows"
if [ -n "$orphans" ]; then
  echo "tests naming an unknown invariant:"
  printf '%s' "$orphans"
fi

bad="$(tr -d '[:space:]' < "$work/bad.txt")"
if [ "${bad:-0}" != "0" ] || [ -n "$orphans" ] || [ -n "$dups" ] || [ -n "$missing_rows" ]; then
  echo "check_invariants: failed (rows to fix: ${bad:-0})" >&2
  exit 1
fi
echo "check_invariants: ok"
