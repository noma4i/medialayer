#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
python3 - "$root" <<'PY'
import hashlib
import re
import sys
from collections import defaultdict
from pathlib import Path

root = Path(sys.argv[1])
source = root / "core" / "src"
allowlist_path = root / "scripts" / "zoo_allowlist.txt"
allowed = {}
if allowlist_path.exists():
    for line in allowlist_path.read_text().splitlines():
        body, _, reason = line.partition("#")
        body = body.strip()
        if body:
            allowed[body] = reason.strip()
            if not reason.strip():
                print(f"zoo allowlist entry without a reason: {body}")
                sys.exit(1)

TEST_FILE = re.compile(r"(_tests\.rs$|/tests?/|test_support\.rs$|test_client\.rs$|/mock\w*\.rs$|memory_store\.rs$|^seeded\.rs$|hardening_tests/)")
files = sorted(source.rglob("*.rs"))


def rel(path):
    return str(path.relative_to(source))


def is_test(path):
    return bool(TEST_FILE.search(rel(path)))


def strip_comments(text):
    return re.sub(r"//[^\n]*", "", text)


production = [path for path in files if not is_test(path)]
texts = {path: strip_comments(path.read_text()) for path in files}
findings = []

# Z1 duplicate public type names in production code.
definitions = defaultdict(list)
declaration = re.compile(r"^\s*pub(?:\([a-z]+\))? (?:struct|enum|trait|type) ([A-Z][A-Za-z0-9]*)", re.M)
for path in production:
    for name in declaration.findall(texts[path]):
        definitions[name].append(rel(path))
for name, places in sorted(definitions.items()):
    if len(places) > 1 and f"type {name}" not in allowed:
        findings.append(f"Z1 type {name} is defined in {len(places)} places: {', '.join(sorted(places))}")

# Z2 banned suffix and prefix words in declared names of production code, and in file names.
BANNED = ["manager", "service", "handler", "helper", "util", "provider", "wrapper", "engine", "processor", "coordinator", "factory"]
named = re.compile(r"\b(?:struct|enum|trait|type|fn|const|static|mod|let|impl)\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)")


def words(identifier):
    snake = re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", identifier).lower()
    return set(snake.split("_"))


for path in production:
    seen = set()
    for name in named.findall(texts[path]):
        hit = words(name) & set(BANNED)
        if hit and name not in seen and f"name {name}" not in allowed:
            seen.add(name)
            findings.append(f"Z2 name {name} in {rel(path)} carries the banned word {sorted(hit)[0]}")
    for part in rel(path).replace(".rs", "").split("/"):
        hit = words(part) & set(BANNED)
        if hit and f"name {part}" not in allowed:
            findings.append(f"Z2 file or module name {part} in {rel(path)} carries the banned word {sorted(hit)[0]}")

# Z3 duplicated lists, constants and function bodies.
string_list = re.compile(r"\[\s*((?:\"[^\"\\]*\"\s*,\s*){3,}\"[^\"\\]*\"\s*,?)\s*\]")
lists = defaultdict(set)
for path in production:
    for match in string_list.findall(texts[path]):
        items = frozenset(re.findall(r"\"([^\"\\]*)\"", match))
        lists[items].add(rel(path))
for items, places in lists.items():
    if len(places) > 1 and f"list {'|'.join(sorted(items))}" not in allowed:
        findings.append(f"Z3 the list {sorted(items)} is written in {len(places)} files: {', '.join(sorted(places))}")

constant = re.compile(r"^\s*(?:pub(?:\([a-z]+\))? )?const ([A-Z][A-Z0-9_]*): [^=]+= ([^;]+);", re.M)
constants = defaultdict(set)
for path in production:
    for name, value in constant.findall(texts[path]):
        constants[(name, " ".join(value.split()))].add(rel(path))
for (name, value), places in sorted(constants.items()):
    if len(places) > 1 and f"const {name}" not in allowed:
        findings.append(f"Z3 the constant {name} = {value} is written in {len(places)} files: {', '.join(sorted(places))}")

function = re.compile(r"^(\s*)(?:pub(?:\([a-z]+\))? )?(?:async )?fn ([a-z_][a-z0-9_]*)[^\n]*\{\n(.*?)^\1\}", re.M | re.S)
bodies = defaultdict(set)
for path in files:
    for _, name, body in function.findall(texts[path]):
        if name == "from":
            continue
        lines = [line.strip() for line in body.splitlines() if line.strip()]
        if len(lines) >= 5:
            digest = hashlib.sha1("\n".join(lines).encode()).hexdigest()
            bodies[digest].add((name, rel(path)))
for places in bodies.values():
    names = sorted(places)
    if len({path for _, path in names}) > 1 and f"body {names[0][0]}" not in allowed:
        findings.append("Z3 the same function body in several files: " + ", ".join(f"{name} ({path})" for name, path in names))

generators = [rel(path) for path in files if "6364136223846793005" in texts[path] or "0x9e37_79b9" in texts[path].lower() and rel(path) != "seeded.rs"]
if generators and "generator" not in allowed:
    findings.append(f"Z3 a hand-written random generator outside seeded.rs: {', '.join(generators)}")

# Z4 background tasks need an owner: AbortOnDrop, the proxy task set, a JoinSet, an awaited call, or a counted allowlist entry.
spawn = re.compile(r"(?:\btokio::spawn|\.spawn|\bspawn_blocking|\.spawn_blocking)\(")
OWNED_BY = ("AbortOnDrop::new(", "tasks.spawn(", "connections.spawn(", "set.spawn(")


def statement_end(text, start):
    depth = 0
    for index in range(start, len(text)):
        if text[index] == "(":
            depth += 1
        elif text[index] == ")":
            depth -= 1
            if depth == 0:
                return index + 1
    return len(text)


def statement_start(text, position):
    for index in range(position - 1, -1, -1):
        if text[index] in ";{}":
            return index + 1
    return 0


unowned = defaultdict(int)
sites = defaultdict(list)
for path in production:
    text = texts[path]
    for match in spawn.finditer(text):
        begin = statement_start(text, match.start())
        statement = text[begin: statement_end(text, match.end() - 1)]
        if any(owner in statement for owner in OWNED_BY):
            continue
        end = statement_end(text, match.end() - 1)
        if text[end: end + 60].lstrip().startswith(".await"):
            continue
        binding = re.match(r"\s*let (?:mut )?(\w+) =", statement)
        if binding and re.search(r"(?:AbortOnDrop::new\(\s*" + binding.group(1) + r"\s*\)|\b" + binding.group(1) + r"\.abort\(\))", text[end: end + 800]):
            continue
        unowned[rel(path)] += 1
        sites[rel(path)].append(text.count("\n", 0, match.start()) + 1)
for path, count in sorted(unowned.items()):
    allowed_count = None
    for key in allowed:
        parts = key.split()
        if parts[:2] == ["spawn", path]:
            allowed_count = int(parts[2])
    if allowed_count is None:
        findings.append(f"Z4 {path} starts {count} task(s) with no owner at lines {sites[path]} (AbortOnDrop, proxy task set, JoinSet, awaited call or allowlist entry)")
    elif count > allowed_count:
        findings.append(f"Z4 {path} starts {count} unowned tasks, the allowlist grants {allowed_count}")
for key in allowed:
    parts = key.split()
    if parts[0] == "spawn" and unowned.get(parts[1], 0) < int(parts[2]):
        findings.append(f"Z4 the allowlist grants {parts[2]} unowned tasks to {parts[1]} but it starts {unowned.get(parts[1], 0)}: lower the entry")

# Z5 temporary and print code in production. trace.rs is the ML_TRACE sink and prints by design.
for path in production:
    if rel(path) == "trace.rs":
        continue
    text = path.read_text()
    for number, line in enumerate(text.splitlines(), 1):
        if "TEMPORARY" in line or re.search(r"\b(?:dbg|println|eprintln)!\(", strip_comments(line)):
            findings.append(f"Z5 {rel(path)}:{number} temporary or print code in production")

# Z6 repeating timers and sleeps in the wrappers (INV-IDLE-05): each one needs an allowlist entry `periodic <path> <count>` with its period and owner.
PERIODIC = {".swift": re.compile(r"addPeriodicTimeObserver|scheduledTimer|Timer\.publish|Task\.sleep"), ".kt": re.compile(r"postDelayed|(?<![.\w])delay\(|scheduleAtFixedRate|fixedRateTimer")}
wrapper_roots = [root / "MediaLayer" / "Sources", root / "kotlin" / "src" / "main", root / "kotlin" / "ui" / "src" / "main"]
periodic = {}
for base in wrapper_roots:
    for path in sorted(base.rglob("*")) if base.exists() else []:
        if path.suffix not in (".swift", ".kt"):
            continue
        count = len(PERIODIC[path.suffix].findall(re.sub(r"//[^\n]*", "", path.read_text())))
        if count:
            periodic[str(path.relative_to(root))] = count
for path, count in sorted(periodic.items()):
    granted = next((int(key.split()[2]) for key in allowed if key.split()[:2] == ["periodic", path]), 0)
    if count != granted:
        findings.append(f"Z6 {path} has {count} repeating timer or sleep call(s), the allowlist grants {granted}")
for key in allowed:
    parts = key.split()
    if parts[0] == "periodic" and parts[1] not in periodic:
        findings.append(f"Z6 the allowlist grants {parts[2]} timers to {parts[1]} but it has none: remove the entry")

# Z7 the playback readahead belongs to the core: no buffer length in the wrappers, no second buffer policy in config.
DEAD_BUFFER_NAMES = ["warmBuffer", "coldBuffer", "buffer_seconds", "low_buffer_seconds", "bufferSeconds"]
wrapper_files = [*(root / "MediaLayer" / "Sources").rglob("*.swift"), *(root / "kotlin").glob("**/src/main/**/*.kt")]
for path in [*production, *wrapper_files]:
    text = strip_comments(path.read_text())
    place = rel(path) if path in production else str(path.relative_to(root))
    for name in DEAD_BUFFER_NAMES:
        if re.search(rf"\b{name}\b", text):
            findings.append(f"Z7 {place} names {name}: the readahead comes from the core (set_readahead), a buffer policy in config or a wrapper is a second copy")
    if path in wrapper_files and re.search(r"preferredForwardBufferDuration\s*=\s*[0-9]|setBufferDurationsMs\([^)]*\b[0-9]", text):
        findings.append(f"Z7 {place} sets a buffer length by a literal: only the core hint may set it")

# Z8 a phone library never panics on the environment: a production panic call needs `panic:<file>:<fragment>` in the allowlist with the reason it cannot fail.
PANIC_CALL = re.compile(r"\.(?:expect|unwrap)\(|\b(?:panic|unreachable|todo|unimplemented)!\(")
test_gated = set()
for path in files:
    for gated in re.finditer(r"#\[cfg\(test\)\]\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+(\w+)\s*;", texts[path]):
        base = path.parent if path.name in ("mod.rs", "lib.rs") else path.parent / path.stem
        test_gated.update(candidate for candidate in (base / f"{gated.group(1)}.rs", base / gated.group(1) / "mod.rs") if candidate.exists())
panic_allowed = [entry[len("panic:"):].split(":", 1) for entry in allowed if entry.startswith("panic:")]
panic_used = set()
for path in files:
    if is_test(path) or path in test_gated:
        continue
    body = texts[path]
    inline_tests = re.search(r"#\[cfg\(test\)\]\s*mod\s+\w+\s*\{", body)
    for number, line in enumerate((body[: inline_tests.start()] if inline_tests else body).splitlines(), 1):
        if not PANIC_CALL.search(line):
            continue
        granted = [entry for entry in panic_allowed if entry[0] == rel(path) and entry[1] in line]
        panic_used.update(map(tuple, granted))
        if not granted:
            findings.append(f"Z8 a phone library never panics on the environment, return an error or a degraded answer: {rel(path)}:{number}: {line.strip()}")
for entry in panic_allowed:
    if tuple(entry) not in panic_used:
        findings.append(f"Z8 the allowlist grants a panic call to {entry[0]} matching `{entry[1]}` but the code has none: remove the entry")

if findings:
    print("zoo check found", len(findings), "problem(s):")
    for finding in findings:
        print(" -", finding)
    sys.exit(1)
print("zoo check: clean")
PY
