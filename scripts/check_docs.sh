#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
python3 - "$root" <<'PY'
import re
import sys
from collections import defaultdict
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


# Examples must use the real API: names, constructors and labels are looked up in the platform sources.
import glob

def read_all(patterns):
    parts = []
    for pattern in patterns:
        for path in glob.glob(str(root / pattern), recursive=True):
            if "/build/" not in path:
                parts.append(Path(path).read_text())
    return "\n".join(parts)

platform = {
    "swift": read_all(["swift/Sources/**/*.swift"]),
    "kotlin": read_all(["kotlin/src/main/**/*.kt", "kotlin/ui/src/main/**/*.kt"]),
}
declaration = {
    "swift": re.compile(r"\b(?:struct|class|enum|protocol|typealias|actor)\s+(\w+)"),
    "kotlin": re.compile(r"\b(?:class|interface|object|typealias)\s+(\w+)"),
}
function_declaration = {
    "swift": re.compile(r"\b(?:func|var|let|case)\s+`?(\w+)"),
    "kotlin": re.compile(r"\b(?:fun\s+(?:[\w<>?.]+\.)?|val\s+|var\s+)`?(\w+)"),
}
family = re.compile(r"\b(Media[A-Z]\w*|OpenConfig|DeviceMemory|\w+Fetcher|\w+SlotFactory|PlayerEvents|MemoryReleaser|Placeholder|PlaybackRoute)\b")
free_functions = ["defaultLimits", "defaultPoolSettings", "decodeBlurhash"]
labelled = ["OpenConfig", "DeviceMemory", "MediaVideo", "MediaImage", "MediaAudio"]
unpaired_free = set()


def window(text, name, kind, width=4000):
    spans = []
    for match in re.finditer(r"\b%s\s+`?%s\b" % (kind, re.escape(name)), text):
        spans.append(text[match.start(): match.start() + width])
    return spans


def top_level_labels(call_text, language):
    depth, labels, current = 0, [], ""
    pattern = re.compile(r"^\s*(\w+)\s*:(?!:)") if language == "swift" else re.compile(r"^\s*(\w+)\s*=(?!=)")
    pieces = []
    for char in call_text:
        if char in "([{":
            depth += 1
        elif char in ")]}":
            depth -= 1
        if char == "," and depth == 1:
            pieces.append(current)
            current = ""
        else:
            current += char
    pieces.append(current)
    for piece in pieces:
        match = pattern.match(piece.lstrip("("))
        if match:
            labels.append(match.group(1))
    return labels


if platform["swift"] and "public enum MediaState" in platform["swift"] and platform["kotlin"] and "sealed class MediaState" in platform["kotlin"]:
    for document in documents:
        text = document.read_text()
        name = document.relative_to(root)
        for language, body in re.findall(r"^```(swift|kotlin)\n(.*?)^```", text, re.M | re.S):
            source_text = platform[language]
            own = set(declaration[language].findall(body))
            functions = set(function_declaration[language].findall(source_text))
            types = set(declaration[language].findall(source_text))
            for method in set(re.findall(r"\bmedia\.(\w+)", body)):
                if method not in functions:
                    problems.append(f"{name}: example calls media.{method} but no {language} source declares it")
            for token in set(family.findall(body)) - own:
                if token not in types and token not in functions:
                    problems.append(f"{name}: example uses {token} but no {language} source declares it")
            for function in free_functions:
                if re.search(r"\b%s\(" % function, body) and function not in functions:
                    problems.append(f"{name}: example calls {function} but no {language} source declares it")
            for owner in labelled:
                for match in re.finditer(r"\b%s\(" % owner, body):
                    depth, end = 0, match.end() - 1
                    for index in range(end, len(body)):
                        depth += body[index] == "("
                        depth -= body[index] == ")"
                        if depth == 0:
                            end = index + 1
                            break
                    spans = window(source_text, owner, r"(?:struct|class|enum|interface|object)") or []
                    if not spans:
                        continue
                    for label in top_level_labels(body[match.end() - 1: end], language):
                        if not any(re.search(r"\b%s\b" % re.escape(label), span) for span in spans):
                            problems.append(f"{name}: example passes {label} to {owner} but the {language} declaration has no such parameter")
            for dotted_owner, dotted in set(re.findall(r"\b(Evict|Kind|MediaState)\.(\w+)\b", body)):
                spans = window(source_text, dotted_owner, r"(?:enum|class|sealed class|enum class)", 6000)
                if spans and not any(re.search(r"\b%s\b" % re.escape(dotted), span) for span in spans):
                    problems.append(f"{name}: example uses {dotted_owner}.{dotted} but the {language} declaration has no such case")

# UNBUILT markers: every marked doc is listed with its invariant, every listed invariant is UNBUILT in the local spec.
listed = defaultdict(list)
unbuilt_list = root / "scripts" / "docs_unbuilt.txt"
for line in unbuilt_list.read_text().splitlines():
    if line.strip() and not line.startswith("#"):
        path, invariant = line.split()
        listed[path].append(invariant)
for document in documents:
    name = str(document.relative_to(root))
    marked = "UNBUILT" in document.read_text()
    if marked and name not in listed:
        problems.append(f"{name}: carries UNBUILT but is missing from scripts/docs_unbuilt.txt")
    if name in listed and not marked:
        problems.append(f"{name}: is listed in scripts/docs_unbuilt.txt but carries no UNBUILT marker")
registry = root / "specs" / "invariants.md"
if registry.exists():
    rows = dict(re.findall(r"^\| (INV-[A-Z]+-\d+) \| (REQUIRED|UNBUILT|OPEN) \|", registry.read_text(), re.M))
    for name, invariants in listed.items():
        for invariant in invariants:
            if rows.get(invariant) != "UNBUILT":
                problems.append(f"{name}: lists {invariant} which is {rows.get(invariant, 'missing')} in specs/invariants.md, not UNBUILT")

if problems:
    print("docs check found", len(problems), "problem(s):")
    for problem in problems:
        print(" -", problem)
    sys.exit(1)
print("docs check: clean")
PY
