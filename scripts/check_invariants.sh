#!/usr/bin/env bash
# One registry of invariant numbers. Every inv_<area>_NN test of Rust, Swift, Kotlin and shell has one INV-<AREA>-NN row in
# specs/invariants.md, every REQUIRED row has a test, a row names only platforms that have the test, no ID or promise text
# repeats inside one area, and every registry row has a status. The specs folder is not in git: without it the check
# prints why it is skipped and passes, so brick and CI are not broken.
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
specs="${MEDIALAYER_SPECS:-}"
if [ -z "$specs" ] && [ -d "$root/specs" ]; then
  specs="$root/specs"
fi
if [ -z "$specs" ]; then
  common="$(git -C "$root" rev-parse --git-common-dir 2>/dev/null || true)"
  if [ -n "$common" ] && [ -d "$(cd -- "$root" && cd -- "$common/.." && pwd -P)/specs" ]; then
    specs="$(cd -- "$root" && cd -- "$common/.." && pwd -P)/specs"
  fi
fi
if [ -z "$specs" ] || [ ! -f "$specs/invariants.md" ]; then
  echo "check_invariants: skipped, no specs folder (set MEDIALAYER_SPECS or keep specs/ next to the repository)"
  exit 0
fi

python3 - "$root" "$specs" <<'PY'
import collections
import glob
import re
import sys
from pathlib import Path

root, specs = Path(sys.argv[1]), Path(sys.argv[2])
sources = [("core/**/*.rs", "Rust"), ("gen/**/*.rs", "Rust"), ("MediaLayer/**/*.swift", "Swift"),
           ("kotlin/**/*.kt", "Kotlin"), ("scripts/*.sh", "Shell")]
test_pattern = re.compile(r"(?<![a-z0-9])inv_([a-z]+)_(\d{1,3})(?![0-9])")
tests = collections.defaultdict(set)
for pattern, platform in sources:
    for path in glob.glob(str(root / pattern), recursive=True):
        if "/build/" in path or "/target" in path or path.endswith("check_invariants.sh"):
            continue
        for match in test_pattern.finditer(Path(path).read_text(errors="ignore")):
            tests[(match.group(1).upper(), int(match.group(2)))].add(platform)

statuses = {"REQUIRED", "UNBUILT", "OPEN", "REPLACED"}
row_pattern = re.compile(r"^\| INV-([A-Z0-9]+)-(\d+) \|")
rows = collections.defaultdict(list)
problems = []
for path in sorted(specs.glob("*.md")):
    for number, line in enumerate(path.read_text().split("\n"), 1):
        match = row_pattern.match(line)
        if not match:
            continue
        cells = [cell.strip() for cell in re.split(r"(?<!\\)\|", line)[1:-1]]
        if len(cells) < 8:
            continue  # a row of the mutation table or of the old rule table: not a registry row
        key = (match.group(1), int(match.group(2)))
        rows[key].append((path.name, number, cells))
        if cells[1] not in statuses:
            problems.append(f"{path.name}:{number}: INV-{key[0]}-{key[1]:02d} has no status in the second column")

for key, found in sorted(rows.items()):
    label = f"INV-{key[0]}-{key[1]:02d}"
    if len(found) > 1:
        problems.append(f"{label}: {len(found)} registry rows ({', '.join(f'{n}:{l}' for n, l, _ in found)}), one source only")
    name, number, cells = found[0]
    present = tests.get(key, set())
    if cells[1] == "REQUIRED" and not present:
        problems.append(f"{name}:{number}: {label} is REQUIRED and no test is named inv_{key[0].lower()}_{key[1]:02d}_*")
    for platform, named in re.findall(r"(Rust|Swift|Kotlin)\s+`inv_([a-z]+_\d+)", cells[4]):
        area, count = named.split("_")
        if (area.upper(), int(count)) == key and platform not in present:
            problems.append(f"{name}:{number}: {label} names a {platform} test and {platform} has none")

for key in sorted(tests):
    if key not in rows:
        problems.append(f"inv_{key[0].lower()}_{key[1]:02d}_* ({', '.join(sorted(tests[key]))}) has no INV-{key[0]}-{key[1]:02d} row in the specs")

promises = collections.defaultdict(list)
for key, found in rows.items():
    text = re.sub(r"[^a-z0-9]+", " ", found[0][2][2].lower()).strip()
    if len(text) > 25 and found[0][2][1] != "REPLACED":
        promises[(key[0], text)].append(key[1])
for (area, _), numbers in sorted(promises.items()):
    if len(numbers) > 1:
        problems.append(f"INV-{area}: numbers {', '.join(f'{n:02d}' for n in sorted(numbers))} state one promise, one row per promise")

if problems:
    print("\n".join(problems))
    print(f"check_invariants: {len(problems)} problem(s)")
    sys.exit(1)
print(f"check_invariants: {len(rows)} rows, {len(tests)} numbered tests, one registry")
PY
