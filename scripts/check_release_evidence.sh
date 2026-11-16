#!/usr/bin/env bash
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
python3 - "$root" <<'PY'
import os
import hashlib
import io
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import zipfile

source = Path(sys.argv[1])
with tempfile.TemporaryDirectory() as temporary:
    base = Path(temporary)
    root = base / "repo"
    (root / "scripts").mkdir(parents=True)
    for name in ("release_evidence.py", "check_medialayer.sh", "check_steps.sh"):
        shutil.copy(source / "scripts" / name, root / "scripts" / name)
    (root / "scripts/check_api_twins.py").write_text("def compare(*args): return []\n")
    (root / ".gitignore").write_text("target/\n**/build/\n")
    swift = root / "MediaLayer/Sources/MediaLayer/medialayer.swift"
    kotlin = root / "kotlin/src/main/kotlin/com/noma4i/medialayer/medialayer.kt"
    for path in (swift, kotlin):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(path.suffix)
    env = dict(os.environ, GIT_AUTHOR_NAME="test", GIT_AUTHOR_EMAIL="test@example.invalid", GIT_COMMITTER_NAME="test", GIT_COMMITTER_EMAIL="test@example.invalid")
    def git(*args):
        return subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True, text=True, env=env).stdout.strip()
    def evidence(*args, success=True):
        result = subprocess.run(["python3", str(root / "scripts/release_evidence.py"), *map(str, args)], capture_output=True, text=True)
        if (result.returncode == 0) != success:
            raise AssertionError(f"{args}: {result.stderr}")
        return result.stdout.strip()
    git("init")
    git("add", ".")
    git("commit", "-m", "Add sources")
    source_tree = evidence("tree")
    schema = root / "target/api-schema/swift"
    schema.mkdir(parents=True)
    for module in ("MediaLayer", "MediaLayerUI", "MediaLayerTesting"):
        (schema / f"{module}.json").write_text('{"module":{"name":"' + module + '"}}')
    (schema.parent / "swift-path").write_text(str(schema))
    for directory in ("kotlin", "kotlin/ui", "kotlin/testing"):
        output = root / directory / "build/api/public.api"
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text("schema")
    android, stage = base / "android", base / "stage"
    android.mkdir()
    stage.mkdir()
    with zipfile.ZipFile(android / "medialayer-android.zip", "w") as archive:
        archive.write(kotlin, "src/main/kotlin/com/noma4i/medialayer/medialayer.kt")
    shutil.copy(android / "medialayer-android.zip", stage / "medialayer-android.zip")
    shutil.copy(swift, stage / "medialayer.swift")
    evidence("android", android, source_tree)
    evidence("stage", stage, source_tree)
    def lane(name, tree_id):
        evidence("lane", name, base / f"{name}.json", tree_id, *evidence("steps", name).split())
    lane("mac", source_tree)
    lane("brick", source_tree)
    args = (base / "mac.json", base / "brick.json", android, stage, base / "checks.json")
    evidence("merge", *args)
    evidence("verify", base / "checks.json", android, stage, "HEAD")
    (android / "medialayer-android.zip").write_bytes(b"other archive")
    evidence("verify", base / "checks.json", android, stage, "HEAD", success=False)
    shutil.copy(stage / "medialayer-android.zip", android / "medialayer-android.zip")
    swift.write_text("other source")
    git("add", ".")
    git("commit", "-m", "Change source")
    evidence("merge", *args, success=False)
    evidence("verify", base / "checks.json", android, stage, "HEAD", success=False)
    changed_tree = evidence("tree")
    lane("mac", changed_tree)
    shutil.copy(swift, stage / "medialayer.swift")
    evidence("stage", stage, changed_tree)
    evidence("merge", *args)
    evidence("verify", base / "checks.json", android, stage, "HEAD")
    print("ok a Swift-only edit keeps the Kotlin lane report and the Android evidence valid")
    (root / "kotlin/Extra.kt").write_text("changed")
    git("add", ".")
    git("commit", "-m", "Change Kotlin")
    changed_tree = evidence("tree")
    lane("mac", changed_tree)
    evidence("merge", *args, success=False)
    print("ok release evidence rejects changed artifacts and source trees")

with tempfile.TemporaryDirectory() as temporary:
    base = Path(temporary)
    root, android = base / "repo", base / "android"
    (root / "scripts").mkdir(parents=True)
    (root / "core").mkdir()
    (root / "MediaLayer/Sources/MediaLayer").mkdir(parents=True)
    (root / "kotlin/src/main/kotlin/com/noma4i/medialayer").mkdir(parents=True)
    for name in ("release.sh", "release_evidence.py", "check_medialayer.sh", "check_steps.sh"):
        shutil.copy(source / "scripts" / name, root / "scripts" / name)
    (root / "scripts/check_api_twins.py").write_text("def compare(*args): return []\n")
    (root / ".gitignore").write_text("target/\n**/build/\n")
    (root / "core/Cargo.toml").write_text('[package]\nversion = "3.26.1"\n')
    (root / "CHANGELOG.md").write_text("## 3.26.1 - 2026-01-01\n")
    (root / "MediaLayer/Sources/MediaLayer/medialayer.swift").write_text("swift binding")
    (root / "kotlin/src/main/kotlin/com/noma4i/medialayer/medialayer.kt").write_text("kotlin binding")
    env = dict(os.environ, GIT_AUTHOR_NAME="test", GIT_AUTHOR_EMAIL="test@example.invalid", GIT_COMMITTER_NAME="test", GIT_COMMITTER_EMAIL="test@example.invalid")
    def run(*args, success=True):
        result = subprocess.run(list(map(str, args)), cwd=root, env=env, capture_output=True, text=True)
        if (result.returncode == 0) != success:
            raise AssertionError(f"{args}: {result.stdout}\n{result.stderr}")
        return result
    def git(*args):
        return run("git", *args).stdout.strip()
    def evidence(*args):
        return run("python3", root / "scripts/release_evidence.py", *args).stdout.strip()
    git("init", "-b", "master")
    git("add", ".")
    git("commit", "-m", "Add sources")
    git("tag", "3.26.1")
    old_tree = evidence("tree")
    android.mkdir()
    abis = ("arm64-v8a", "armeabi-v7a", "x86_64", "x86")
    with zipfile.ZipFile(android / "medialayer-android.zip", "w") as archive:
        for abi in abis:
            archive.writestr(f"src/main/jniLibs/{abi}/libmedialayer.so", "so")
        archive.writestr("src/main/kotlin/com/noma4i/medialayer/medialayer.kt", "kotlin binding")
    jar_data = io.BytesIO()
    with zipfile.ZipFile(jar_data, "w") as jar:
        jar.writestr("com/noma4i/medialayer/Media.class", "class")
    for name in ("medialayer", "medialayer-ui", "medialayer-testing"):
        directory = android / f"maven/com/noma4i/{name}/3.26.2"
        directory.mkdir(parents=True)
        with zipfile.ZipFile(directory / f"{name}-3.26.2.aar", "w") as aar:
            aar.writestr("classes.jar", jar_data.getvalue())
            if name == "medialayer":
                for abi in abis:
                    aar.writestr(f"jni/{abi}/libmedialayer.so", "so")
        (directory / f"{name}-3.26.2.pom").write_text("<artifactId>medialayer</artifactId>")
        (directory / f"{name}-3.26.2.module").write_text("{}")
    evidence("android", android, old_tree)
    old_evidence = (android / "source-evidence.json").read_bytes()
    (root / "core/Cargo.toml").write_text('[package]\nversion = "3.26.2"\n')
    (root / "CHANGELOG.md").write_text("## 3.26.2 - 2026-01-02\n\nRelease.\n")
    git("add", ".")
    git("commit", "-m", "Release 3.26.2")
    current_tree = evidence("tree")
    schema = root / "target/api-schema/swift"
    schema.mkdir(parents=True)
    for module in ("MediaLayer", "MediaLayerUI", "MediaLayerTesting"):
        (schema / f"{module}.json").write_text('{"module":{"name":"' + module + '"}}')
    (schema.parent / "swift-path").write_text(str(schema))
    for directory in ("kotlin", "kotlin/ui", "kotlin/testing"):
        output = root / directory / "build/api/public.api"
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text("schema")
    stage = root / "target/release-3.26.2"
    stage.mkdir(parents=True)
    shutil.copy(android / "medialayer-android.zip", stage / "medialayer-android.zip")
    (stage / "medialayer.swift").write_text("swift binding")
    (stage / "MediaLayerFFI.xcframework.zip").write_bytes(b"swift archive")
    (stage / "medialayer-gen-macos-universal").write_bytes(b"generator")
    names = ("MediaLayerFFI.xcframework.zip", "medialayer.swift", "medialayer-android.zip", "medialayer-gen-macos-universal")
    (stage / "SHA256SUMS").write_text("".join(f"{hashlib.sha256((stage / name).read_bytes()).hexdigest()}  {name}\n" for name in names))
    evidence("stage", stage, current_tree)
    for name in ("mac", "brick"):
        evidence("lane", name, base / f"{name}.json", current_tree, *evidence("steps", name).split())
    evidence("android", android, current_tree)
    evidence("merge", base / "mac.json", base / "brick.json", android, stage, base / "checks.json")
    (android / "source-evidence.json").write_bytes(old_evidence)
    result = run("bash", root / "scripts/release.sh", "3.26.2", "--android", android, "--report", base / "checks.json", success=False)
    if "Android artifacts do not match their source evidence" not in result.stderr:
        raise AssertionError(f"dry run did not reject the old Android tree: {result.stderr}")
    print("ok release dry run rejects an Android archive from another tree")
    evidence("android", android, current_tree)
    (root / "Package.swift").write_text("package manifest")
    git("add", "Package.swift")
    git("commit", "-m", "Package release 3.26.2")
    result = run("bash", root / "scripts/release.sh", "3.26.2", "--android", android, "--report", base / "checks.json", success=False)
    if "checks report does not match release" in result.stderr or "End-of-central-directory" not in result.stderr:
        raise AssertionError(f"package retry did not reach the archive check: {result.stderr}")
    print("ok package retry accepts the checked source tree")
PY
