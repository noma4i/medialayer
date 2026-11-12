#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
python3 - "$root" <<'PY'
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import zipfile

source = Path(__import__("sys").argv[1])
script = source / "scripts/release.sh"
failed = []

def check_android_build_arguments():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / "scripts").mkdir()
        shutil.copy2(source / "scripts/release_android.sh", root / "scripts/release_android.sh")
        (root / "scripts/build_kotlin.sh").write_text('#!/bin/bash\nprintf "%s\\n" "$#" "$@" > "$(dirname "$0")/arguments"\nexit 73\n')
        result = subprocess.run(["bash", str(root / "scripts/release_android.sh"), "3.26.2", str(root / "out")], capture_output=True, text=True)
        assert result.returncode == 73, f"build stub was not reached: {result.stderr}"
        assert (root / "scripts/arguments").read_text() == "0\n", "Android build must regenerate bindings"
        print("ok Android distribution regenerates bindings")

def run_case(publish):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / "scripts").mkdir()
        shutil.copy2(script, root / "scripts/release.sh")
        (root / "scripts/release.sh").chmod(0o755)
        for name in ("check_docs.sh", "check_zoo.sh"):
            (root / "scripts" / name).write_text("#!/bin/sh\nexit 0\n")
        (root / "scripts/build_ios.sh").write_text("#!/bin/sh\nmkdir -p MediaLayer/MediaLayerFFI.xcframework/ios-arm64 MediaLayer/MediaLayerFFI.xcframework/ios-arm64-simulator MediaLayer/MediaLayerFFI.xcframework/macos-arm64 MediaLayer/Sources/MediaLayer\nprintf bridge > MediaLayer/Sources/MediaLayer/medialayer.swift\nprintf '<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>AvailableLibraries</key><array><dict><key>LibraryIdentifier</key><string>ios-arm64</string><key>SupportedArchitectures</key><array><string>arm64</string></array><key>SupportedPlatform</key><string>ios</string></dict><dict><key>LibraryIdentifier</key><string>ios-arm64-simulator</string><key>SupportedArchitectures</key><array><string>arm64</string></array><key>SupportedPlatform</key><string>ios</string><key>SupportedPlatformVariant</key><string>simulator</string></dict><dict><key>LibraryIdentifier</key><string>macos-arm64</string><key>SupportedArchitectures</key><array><string>arm64</string></array><key>SupportedPlatform</key><string>macos</string></dict></array></dict></plist>\n' > MediaLayer/MediaLayerFFI.xcframework/Info.plist\nprintf binary > MediaLayer/MediaLayerFFI.xcframework/ios-arm64/libmedialayer.a\nprintf binary > MediaLayer/MediaLayerFFI.xcframework/ios-arm64-simulator/libmedialayer.a\nprintf binary > MediaLayer/MediaLayerFFI.xcframework/macos-arm64/libmedialayer.a\n")
        (root / "scripts/build_ios.sh").chmod(0o755)
        (root / "MediaLayer/Sources/MediaLayerUI").mkdir(parents=True)
        (root / "MediaLayer/Sources/MediaLayerTesting").mkdir(parents=True)
        (root / "MediaLayer/Sources/MediaLayerUI/MediaLayerUI.swift").write_text("public struct MediaLayerUI {}\n")
        (root / "MediaLayer/Sources/MediaLayerTesting/MediaLayerTesting.swift").write_text("public struct MediaLayerTesting {}\n")
        (root / "core").mkdir()
        (root / "gen").mkdir()
        (root / "core/Cargo.toml").write_text('[package]\nname = "medialayer"\nversion = "3.26.2"\n')
        (root / "gen/Cargo.toml").write_text('[package]\nname = "medialayer-gen"\nversion = "3.26.2"\n')
        (root / "CHANGELOG.md").write_text('## 3.26.2 - 2026-10-27\n\n### Fixed\n- Release check.\n\n## Unreleased\n')
        (root / "target/bin").mkdir(parents=True)
        android = root / "android-result"
        archive_root = android / "maven/com/noma4i/medialayer/3.26.2"
        ui_root = android / "maven/com/noma4i/medialayer-ui/3.26.2"
        testing_root = android / "maven/com/noma4i/medialayer-testing/3.26.2"
        archive_root.mkdir(parents=True)
        ui_root.mkdir(parents=True)
        testing_root.mkdir(parents=True)
        with zipfile.ZipFile(android / "medialayer-android.zip", "w") as archive:
            for name in ("src/main/jniLibs/arm64-v8a/libmedialayer.so", "src/main/jniLibs/armeabi-v7a/libmedialayer.so", "src/main/jniLibs/x86_64/libmedialayer.so", "src/main/jniLibs/x86/libmedialayer.so", "src/main/kotlin/com/noma4i/medialayer/medialayer.kt"):
                archive.writestr(name, "artifact")
        for directory, name, pom in ((archive_root, "medialayer", "<project/>"), (ui_root, "medialayer-ui", "<dependency><artifactId>medialayer</artifactId></dependency>"), (testing_root, "medialayer-testing", "<dependency><artifactId>medialayer</artifactId></dependency>")):
            with zipfile.ZipFile(directory / f"{name}-3.26.2.aar", "w") as aar:
                aar.writestr("classes.jar", "jar")
                if name == "medialayer":
                    for abi in ("arm64-v8a", "armeabi-v7a", "x86_64", "x86"):
                        aar.writestr(f"jni/{abi}/libmedialayer.so", "so")
            (directory / f"{name}-3.26.2.pom").write_text(pom)
            (directory / f"{name}-3.26.2.module").write_text("{}")
        event_file = root / "events"
        env = dict(os.environ, PATH=f"{root}/target/bin:{os.environ['PATH']}", TEST_ROOT=str(root), TEST_EVENTS=str(event_file))
        stubs = {
            "git": '''#!/usr/bin/env python3
import os, pathlib, shutil, sys
a=sys.argv[1:]
with open(os.environ["TEST_EVENTS"],"a") as f: f.write("git " + " ".join(a) + "\\n")
root=pathlib.Path(os.environ["TEST_ROOT"])
if a[:2]==["-C", str(root/"maven-branch")]:
 b=a[2:]
 if b[:2]==["init","-b"]: pathlib.Path(b[-1]).mkdir(exist_ok=True)
 elif b[:2]==["diff","--cached"]: sys.exit(1)
 elif b[:2]==["remote","get-url"]: print("https://example.invalid/medialayer.git")
elif a[:2]==["remote","get-url"]: print("https://example.invalid/medialayer.git")
elif a[:2]==["ls-remote","--heads"]: pass
if a[0]=="add":
 with open(root/"staged", "a") as f: f.write(a[-1] + "\\n")
elif a[:3]==["commit", "-m", "Package release 3.26.2"]:
 for name in (root/"staged").read_text().splitlines():
  dest=root/"committed"/name
  dest.parent.mkdir(parents=True, exist_ok=True)
  shutil.copy2(root/name, dest)
if a[:2]==["status","--porcelain"]: pass
elif a[:2]==["branch","--show-current"]: print("master")
elif a[:2]==["describe","--tags"]: print("3.26.1")
elif a[:2]==["log","--format=%s"]: print("Release 3.26.2")
elif a[:2]==["log","-1"]: print("Release 3.26.2")
if a[:2]==["-C", str(root/"maven-branch")]:
 b=a[2:]
 if b[:2]==["commit","-m"] or b[:2]==["push","-u"]: pass
''',
            "cargo": '''#!/usr/bin/env python3
import os, pathlib, shutil, sys
a=sys.argv[1:]
with open(os.environ["TEST_EVENTS"],"a") as f: f.write("cargo " + " ".join(a) + "\\n")
t=a[a.index("--target")+1]
p=pathlib.Path("target")/t/"release/medialayer-gen"
p.parent.mkdir(parents=True,exist_ok=True); p.write_text(t)
''',
            "lipo": '''#!/usr/bin/env python3
import os, pathlib, sys
a=sys.argv[1:]
with open(os.environ["TEST_EVENTS"],"a") as f: f.write("lipo " + " ".join(a) + "\\n")
if a[0]=="-create": pathlib.Path(a[a.index("-output")+1]).write_text("universal")
elif a[0]=="-archs":
 p=a[-1]
 print("arm64 x86_64" if "universal" in p else "arm64")
''',
            "zip": '''#!/usr/bin/env python3
import os, pathlib, sys
a=sys.argv[1:]
with open(os.environ["TEST_EVENTS"],"a") as f: f.write("zip\\n")
pathlib.Path(a[-2]).write_text("archive")
''',
            "unzip": '''#!/usr/bin/env python3
import os, pathlib, shutil, sys
a=sys.argv[1:]
with open(os.environ["TEST_EVENTS"],"a") as f: f.write("unzip " + " ".join(a) + "\\n")
if "-Z1" in a:
 if a[-1].endswith("classes.jar"): print("com/noma4i/medialayer/Media.class")
 else: print("MediaLayerFFI.xcframework/Info.plist\\nMediaLayerFFI.xcframework/ios-arm64/libmedialayer.a\\nMediaLayerFFI.xcframework/ios-arm64-simulator/libmedialayer.a\\nMediaLayerFFI.xcframework/macos-arm64/libmedialayer.a")
elif "-d" in a:
 d=pathlib.Path(a[a.index("-d")+1])
 for n in ("Info.plist","ios-arm64/libmedialayer.a","ios-arm64-simulator/libmedialayer.a","macos-arm64/libmedialayer.a"):
  p=d/"MediaLayerFFI.xcframework"/n; p.parent.mkdir(parents=True,exist_ok=True)
  if n == "Info.plist": shutil.copy2(pathlib.Path(os.environ["TEST_ROOT"])/"MediaLayer/MediaLayerFFI.xcframework/Info.plist", p)
  else: p.write_text("binary")
elif a[0]=="-q" and a[-1].endswith("medialayer-android.zip"):
 for n in ("src/main/jniLibs/arm64-v8a/libmedialayer.so","src/main/jniLibs/armeabi-v7a/libmedialayer.so","src/main/jniLibs/x86_64/libmedialayer.so","src/main/jniLibs/x86/libmedialayer.so","src/main/kotlin/com/noma4i/medialayer/medialayer.kt"):
  p=pathlib.Path(n); p.parent.mkdir(parents=True,exist_ok=True); p.write_text("artifact")
elif a[0]=="-q":
 pathlib.Path("classes.jar").write_text("jar")
 if a[-1].endswith(".aar"):
  for abi in ("arm64-v8a","armeabi-v7a","x86_64","x86"):
   p=pathlib.Path("jni")/abi/"libmedialayer.so"; p.parent.mkdir(parents=True,exist_ok=True); p.write_text("so")
''',
            "swift": '''#!/usr/bin/env python3
import hashlib, os, pathlib, sys
a=sys.argv[1:]
with open(os.environ["TEST_EVENTS"],"a") as f: f.write("swift " + " ".join(a) + "\\n")
if a[0:2]==["package","compute-checksum"]: print(hashlib.sha256(pathlib.Path(a[-1]).read_bytes()).hexdigest())
else: print("{}")
''',
            "gh": '''#!/usr/bin/env python3
import os, pathlib, shutil, sys
a=sys.argv[1:]
with open(os.environ["TEST_EVENTS"],"a") as f: f.write("gh " + " ".join(a) + "\\n")
if a[:2]==["release","view"] and "--json" not in a: sys.exit(1)
if a[:2]==["release","view"]: print("MediaLayerFFI.xcframework.zip\\nmedialayer.swift\\nmedialayer-android.zip\\nmedialayer-gen-macos-universal")
if a[:2]==["release","download"]:
 d=pathlib.Path(a[a.index("--dir")+1]); d.mkdir(exist_ok=True)
 stage=pathlib.Path(os.environ["TEST_ROOT"])/"target/release-3.26.2"
 for n in ("MediaLayerFFI.xcframework.zip","medialayer.swift","medialayer-android.zip","medialayer-gen-macos-universal"): shutil.copy2(stage/n,d/n)
''',
        }
        for name, text in stubs.items():
            path = root / "target/bin" / name
            path.write_text(text)
            path.chmod(0o755)
        if not publish:
            hidden = root / "testing-hidden"
            shutil.move(str(android / "maven/com/noma4i/medialayer-testing"), str(hidden))
            refused = subprocess.run(["bash", str(root / "scripts/release.sh"), "3.26.2", "--android", str(android)], cwd="/", env=env, capture_output=True, text=True)
            shutil.move(str(hidden), str(android / "maven/com/noma4i/medialayer-testing"))
            if refused.returncode == 0 or "medialayer-testing" not in refused.stderr:
                raise AssertionError("release accepted a stage without medialayer-testing")
        missing = subprocess.run(["bash", str(root / "scripts/release.sh"), "3.26.2"], cwd="/", env=env, capture_output=True, text=True)
        if missing.returncode == 0 or "usage:" not in missing.stderr:
            raise AssertionError("release accepted a missing --android argument")
        if event_file.exists() and "gh " in event_file.read_text():
            raise AssertionError("gh ran without Android artifacts")
        cmd = ["bash", str(root / "scripts/release.sh"), "3.26.2"] + (["--publish"] if publish else []) + ["--android", str(android)]
        result = subprocess.run(cmd, cwd="/", env=env, capture_output=True, text=True)
        if result.returncode:
            raise AssertionError(f"rc={result.returncode}\nstdout={result.stdout}\nstderr={result.stderr}")
        output = result.stdout
        events = event_file.read_text().splitlines()
        if publish:
            manifest = (root / "committed/Package.swift").read_text()
            if (root / "committed/MediaLayer/Sources/MediaLayer/medialayer.swift").read_text() != "bridge":
                raise AssertionError("committed Swift bindings differ from release artifact")
            sequence = [next(i for i, e in enumerate(events) if e.startswith(prefix)) for prefix in ("git commit ", "git tag ", "git push origin master", "git push origin 3.26.2")]
            sequence.append(next(i for i, e in enumerate(events) if e.startswith("git -C ") and e.endswith("push -u origin maven")))
            sequence.append(next(i for i, e in enumerate(events) if e.startswith("gh release create ")))
            if "git tag 3.26.2" not in events:
                raise AssertionError(f"release tag must be lightweight: {[e for e in events if e.startswith('git tag')]}")
            if sequence != sorted(sequence):
                raise AssertionError(f"publication order is wrong: {sequence}")
            android_check = next(i for i, e in enumerate(events) if e.startswith("unzip -q ") and e.endswith("medialayer-android.zip"))
            if android_check >= sequence[0]:
                raise AssertionError("Android artifacts were checked after the package commit")
            gh_index = next(i for i, e in enumerate(events) if e.startswith("gh "))
            last_archive_check = max(i for i, e in enumerate(events) if e.startswith("lipo "))
            if gh_index <= last_archive_check:
                raise AssertionError("gh ran before archive architecture checks")
            print("ok release publish order follows archive checks")
        else:
            manifest = output
            if any(e.startswith("gh ") or e.startswith("git commit ") or e.startswith("git tag ") or e.startswith("git push ") for e in events):
                raise AssertionError("dry run published or mutated git")
            print("ok release dry run prints package URL and checksum")
        if "https://github.com/noma4i/medialayer/releases/download/3.26.2/MediaLayerFFI.xcframework.zip" not in manifest:
            raise AssertionError("generated package URL missing")
        if "checksum: \"" + hashlib.sha256(b"archive").hexdigest() not in manifest:
            raise AssertionError("generated package checksum missing")

def check_lightweight_tag_clone():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@e", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@e")
        def git(*args, cwd=None):
            return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, env=env)
        remote = root / "remote.git"
        git("init", "--bare", str(remote))
        work = root / "work"
        git("init", "-b", "master", str(work))
        (work / "file").write_text("x")
        git("add", "file", cwd=work)
        git("commit", "-m", "Package release 1.0.0", cwd=work)
        git("tag", "1.0.0", cwd=work)
        git("remote", "add", "origin", f"file://{remote}", cwd=work)
        assert git("push", "origin", "master", "1.0.0", cwd=work).returncode == 0
        assert git("cat-file", "-t", "1.0.0", cwd=work).stdout.strip() == "commit", "tag is not lightweight"
        clone = git("clone", "--branch", "1.0.0", "--depth", "1", f"file://{remote}", str(root / "clone"))
        assert clone.returncode == 0 and "is not a commit" not in clone.stderr, f"clone warned: {clone.stderr}"
        print("ok lightweight tag clones without a warning")

try:
    check_lightweight_tag_clone()
except AssertionError as error:
    failed.append(str(error))
try:
    check_android_build_arguments()
except AssertionError as error:
    failed.append(str(error))
for publish in (False, True):
    try:
        run_case(publish)
    except (AssertionError, StopIteration) as error:
        failed.append(str(error))
if failed:
    raise SystemExit("\n".join(failed))
PY
