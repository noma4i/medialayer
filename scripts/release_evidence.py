#!/usr/bin/env python3
import hashlib
import json
import subprocess
import sys
import zipfile

sys.dont_write_bytecode = True
from check_api_twins import compare
from pathlib import Path

root = Path(__file__).resolve().parent.parent


def tree(ref="HEAD"):
    paths = ["--", ".", ":(exclude)MediaLayer/Sources/MediaLayer/medialayer.swift"]
    subprocess.run(["git", "-C", str(root), "diff", "--quiet", "HEAD", *paths], check=True)
    if subprocess.check_output(["git", "-C", str(root), "status", "--porcelain", *paths], text=True).strip():
        raise ValueError("source tree is not clean")
    return subprocess.check_output(["git", "-C", str(root), "rev-parse", ref + "^{tree}"], text=True).strip()


def lane_steps(lane):
    out = subprocess.check_output(["bash", str(root / "scripts/check_medialayer.sh"), "--list-lane", lane], text=True)
    return [tuple(line.split("|", 1)) for line in out.splitlines() if line]


def input_hash(paths, ref="HEAD"):
    prefixes = paths.split()
    listing = subprocess.check_output(["git", "-C", str(root), "ls-tree", "-r", "-z", ref], text=True).split("\0")
    chosen = [entry for entry in listing if entry and ("*" in prefixes or any(entry.split("\t", 1)[1].startswith(prefix) for prefix in prefixes))]
    return hashlib.sha256("\n".join(chosen).encode()).hexdigest()


def kotlin_input(ref="HEAD"):
    paths = dict(lane_steps("brick"))["kotlin-bindings"]
    return input_hash(paths, ref)


def step_records(lane, ref="HEAD", only=None):
    return [{"name": name, "input_hash": input_hash(paths, ref), "result": "ok"} for name, paths in lane_steps(lane) if only is None or name in only]


def check_steps(report, lane, ref="HEAD"):
    if report.get("lane") != lane or report.get("partial"):
        raise ValueError(f"{lane} lane report is missing or partial")
    recorded = {step.get("name"): step for step in report.get("steps", [])}
    for expected in step_records(lane, ref):
        if recorded.get(expected["name"]) != expected:
            raise ValueError(f"{lane} step {expected['name']} did not pass on the current inputs")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def files(directory, excluded=()):
    result = {str(p.relative_to(directory)): digest(p) for p in sorted(directory.rglob("*")) if p.is_file() and p.name not in excluded}
    if not result:
        raise ValueError(f"no artifacts in {directory}")
    return result


def binding(lane):
    path = ("MediaLayer/Sources/MediaLayer/medialayer.swift" if lane == "mac" else
            "kotlin/src/main/kotlin/com/noma4i/medialayer/medialayer.kt")
    return digest(root / path)


def read(path):
    return json.loads(Path(path).read_text())


def write(path, value):
    Path(path).write_text(json.dumps(value, sort_keys=True, indent=2) + "\n")


def main(args):
    action = args[0]
    if action == "tree":
        print(tree())
    elif action == "inputhash":
        print(input_hash(args[1], *args[2:]))
    elif action == "steps":
        print("\n".join(name for name, _ in lane_steps(args[1])))
    elif action == "reusable":
        lane, output = args[1:]
        try:
            previous = read(output)
        except (OSError, ValueError):
            return
        if previous.get("lane") != lane:
            return
        recorded = {step.get("name"): step for step in previous.get("steps", [])}
        print("\n".join(step["name"] for step in step_records(lane) if recorded.get(step["name"]) == step))
    elif action in ("android", "stage"):
        directory = Path(args[1])
        if tree() != args[2]:
            raise ValueError("source changed during artifact build")
        excluded = {"source-evidence.json", "NOTES.md"} if action == "stage" else {"source-evidence.json"}
        bound = {"tree": args[2]} if action == "stage" else {"input": kotlin_input()}
        write(directory / "source-evidence.json", {**bound, "files": files(directory, excluded)})
    elif action == "lane":
        lane, output, start_tree, *passed = args[1:]
        if lane not in ("mac", "brick") or tree() != start_tree:
            raise ValueError("source changed during checks")
        names = {name for name, _ in lane_steps(lane)}
        if not set(passed).issubset(names):
            raise ValueError("unknown step in the passed list")
        records = step_records(lane, only=set(passed))
        if len(records) < len(names):
            write(output, {"lane": lane, "steps": records, "partial": True})
            print(f"partial report: {len(records)} of {len(names)} steps green")
            sys.exit(3)
        try:
            previous = read(output)
        except (OSError, ValueError):
            previous = {}
        try:
            if lane == "mac":
                schema_dir = Path((root / "target/api-schema/swift-path").read_text().strip())
                schema = [p.read_text() for p in sorted(schema_dir.glob("*.json"))]
            else:
                schema = [(root / path / "build/api/public.api").read_text() for path in ("kotlin", "kotlin/ui", "kotlin/testing")]
        except OSError:
            schema = previous.get("schema") if previous.get("lane") == lane else None
        if lane == "mac":
            modules = {json.loads(item)["module"]["name"] for item in schema or []}
            if not {"MediaLayer", "MediaLayerUI", "MediaLayerTesting"}.issubset(modules):
                raise ValueError("Swift API schema is incomplete")
        if not schema or any(not item.strip() for item in schema):
            raise ValueError("API schema is missing")
        write(output, {"lane": lane, "steps": records, "binding": binding(lane), "schema": schema})
    elif action in ("merge", "verify"):
        if action == "merge":
            mac, brick, android, stage, output = args[1:]
            mac, brick = read(mac), read(brick)
            current = tree()
            check_steps(mac, "mac")
            check_steps(brick, "brick")
            if len(brick["schema"]) != 3:
                raise ValueError("Kotlin API schema is incomplete")
            failures = compare(mac["schema"], brick["schema"], root / "scripts/api-twins.txt")
            print("API comparison uses symbol, owner and arity; JVM erasure omits generic and nullable types.")
            if failures:
                raise ValueError("API symbols without twins:\n" + "\n".join(failures))
            evidence = read(Path(android) / "source-evidence.json")
            android_files = files(Path(android), {"source-evidence.json"})
            if evidence != {"input": kotlin_input(), "files": android_files}:
                raise ValueError("Android artifacts do not match their source evidence")
            with zipfile.ZipFile(Path(android) / "medialayer-android.zip") as archive:
                bridge = archive.read("src/main/kotlin/com/noma4i/medialayer/medialayer.kt")
            if hashlib.sha256(bridge).hexdigest() != brick["binding"]:
                raise ValueError("Android bindings do not match the checked bindings")
            if digest(Path(stage) / "medialayer.swift") != mac["binding"]:
                raise ValueError("Swift bindings do not match the checked bindings")
            if digest(Path(stage) / "medialayer-android.zip") != digest(Path(android) / "medialayer-android.zip"):
                raise ValueError("release stage does not contain the checked Android archive")
            stage_files = files(Path(stage), {"NOTES.md", "source-evidence.json"})
            if read(Path(stage) / "source-evidence.json") != {"tree": current, "files": stage_files}:
                raise ValueError("release stage does not match its source evidence")
            write(output, {"tree": current, "android_input": kotlin_input(), "mac": mac["steps"], "brick": brick["steps"], "mac_binding": mac["binding"], "brick_binding": brick["binding"], "android": android_files, "stage": stage_files})
        else:
            report, android, stage, ref = args[1:]
            report = read(report)
            if report["tree"] != tree(ref) or report["mac_binding"] != binding("mac"):
                raise ValueError("report does not match source or Swift bindings")
            check_steps({"lane": "mac", "steps": report["mac"]}, "mac", ref)
            check_steps({"lane": "brick", "steps": report["brick"]}, "brick", ref)
            if report["android_input"] != kotlin_input(ref):
                raise ValueError("report does not match the Kotlin inputs")
            if report["android"] != files(Path(android), {"source-evidence.json"}) or report["stage"] != files(Path(stage), {"NOTES.md", "source-evidence.json"}):
                raise ValueError("report does not match release artifacts")
            evidence = read(Path(android) / "source-evidence.json")
            if evidence != {"input": report["android_input"], "files": report["android"]}:
                raise ValueError("Android artifacts do not match their source evidence")
            if read(Path(stage) / "source-evidence.json") != {"tree": report["tree"], "files": report["stage"]}:
                raise ValueError("release stage does not match its source evidence")
    else:
        raise ValueError("unknown evidence action")


try:
    main(sys.argv[1:])
except (IndexError, KeyError, OSError, ValueError, zipfile.BadZipFile, subprocess.CalledProcessError) as error:
    sys.exit(f"release evidence: {error}")
