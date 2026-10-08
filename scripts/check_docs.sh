#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
python3 - "$root" <<'PY'
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
documents = [root / "README.md", *sorted((root / "docs").glob("*.md"))]
problems = []

banned_words = [
    "layer", "runtime", "contract", "pipeline", "facade", "boundary", "gate", "surface",
    "wrapper", "provider", "manager", "service", "handler", "helper", "util", "engine",
]
status_words = ["in development", "work in progress", "planned", "coming soon", "not finished", "todo", "status:"]
path_roots = ("core/", "gen/", "fixtures/", "swift/", "kotlin/", "scripts/", "docs/")

for document in documents:
    text = document.read_text()
    name = document.relative_to(root)
    lowered = text.lower()
    swift_blocks = len(re.findall(r"^```swift", text, re.M))
    kotlin_blocks = len(re.findall(r"^```kotlin", text, re.M))
    if swift_blocks != kotlin_blocks:
        problems.append(f"{name}: {swift_blocks} swift examples but {kotlin_blocks} kotlin examples, they come in pairs")
    for word in banned_words:
        if re.search(rf"\b{word}\b", lowered):
            problems.append(f"{name}: banned word '{word}'")
    for phrase in status_words:
        if phrase in lowered:
            problems.append(f"{name}: status wording '{phrase}'")
    for match in re.findall(r"`([A-Za-z0-9_./-]+)`", text):
        if match.startswith(path_roots) and not any(char in match for char in "*<>"):
            if not (root / match.rstrip("/")).exists() and not match.startswith("swift/MediaLayerFFI"):
                problems.append(f"{name}: path does not exist: {match}")
    for target in re.findall(r"\]\(([^)#]+\.md)(?:#[^)]*)?\)", text):
        if not (document.parent / target).exists():
            problems.append(f"{name}: broken link: {target}")

if problems:
    print("docs check found", len(problems), "problem(s):")
    for problem in problems:
        print(" -", problem)
    sys.exit(1)
print("docs check: clean")
PY
