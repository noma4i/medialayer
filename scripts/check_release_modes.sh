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

source = Path(__import__("sys").argv[1])
script = source / "scripts/release.sh"
failed = []

def run_case(publish):
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / "scripts").mkdir()
        shutil.copy2(script, root / "scripts/release.sh")
        (root / "scripts/release.sh").chmod(0o755)
        for name in ("check_docs.sh", "check_invariants.sh", "check_zoo.sh"):
            (root / "scripts" / name).write_text("#!/bin/sh\nexit 0\n")
        (root / "scripts/build_ios.sh").write_text("#!/bin/sh\nmkdir -p MediaLayer/MediaLayerFFI.xcframework/ios-arm64 MediaLayer/MediaLayerFFI.xcframework/ios-arm64-simulator MediaLayer/Sources/MediaLayer\nprintf bridge > MediaLayer/Sources/MediaLayer/medialayer.swift\nprintf info > MediaLayer/MediaLayerFFI.xcframework/Info.plist\nprintf binary > MediaLayer/MediaLayerFFI.xcframework/ios-arm64/libmedialayer.a\nprintf binary > MediaLayer/MediaLayerFFI.xcframework/ios-arm64-simulator/libmedialayer.a\n")
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
        event_file = root / "events"
        env = dict(os.environ, PATH=f"{root}/target/bin:{os.environ['PATH']}", TEST_ROOT=str(root), TEST_EVENTS=str(event_file))
        stubs = {
            "git": '''#!/usr/bin/env python3
import os, pathlib, shutil, sys
a=sys.argv[1:]
with open(os.environ["TEST_EVENTS"],"a") as f: f.write("git " + " ".join(a) + "\\n")
root=pathlib.Path(os.environ["TEST_ROOT"])
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
''',
            "cargo": '''#!/usr/bin/env python3
import os, pathlib, sys
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
import os, pathlib, sys
a=sys.argv[1:]
with open(os.environ["TEST_EVENTS"],"a") as f: f.write("unzip " + " ".join(a) + "\\n")
if "-Z1" in a: print("MediaLayerFFI.xcframework/Info.plist\\nMediaLayerFFI.xcframework/ios-arm64/libmedialayer.a\\nMediaLayerFFI.xcframework/ios-arm64-simulator/libmedialayer.a")
elif "-d" in a:
 d=pathlib.Path(a[a.index("-d")+1])
 for n in ("Info.plist","ios-arm64/libmedialayer.a","ios-arm64-simulator/libmedialayer.a"):
  p=d/"MediaLayerFFI.xcframework"/n; p.parent.mkdir(parents=True,exist_ok=True); p.write_text("binary")
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
if a[:2]==["release","view"]: print("MediaLayerFFI.xcframework.zip\\nmedialayer.swift\\nmedialayer-gen-macos-universal")
if a[:2]==["release","download"]:
 d=pathlib.Path(a[a.index("--dir")+1]); d.mkdir(exist_ok=True)
 stage=pathlib.Path(os.environ["TEST_ROOT"])/"target/release-3.26.2"
 for n in ("MediaLayerFFI.xcframework.zip","medialayer.swift","medialayer-gen-macos-universal"): shutil.copy2(stage/n,d/n)
''',
        }
        for name, text in stubs.items():
            path = root / "target/bin" / name
            path.write_text(text)
            path.chmod(0o755)
        cmd = ["bash", str(root / "scripts/release.sh"), "3.26.2"] + (["--publish"] if publish else [])
        result = subprocess.run(cmd, cwd="/", env=env, capture_output=True, text=True)
        if result.returncode:
            raise AssertionError(f"rc={result.returncode}\nstdout={result.stdout}\nstderr={result.stderr}")
        output = result.stdout
        events = event_file.read_text().splitlines()
        if publish:
            manifest = (root / "committed/Package.swift").read_text()
            if (root / "committed/MediaLayer/Sources/MediaLayer/medialayer.swift").read_text() != "bridge":
                raise AssertionError("committed Swift bindings differ from release artifact")
            sequence = [next(i for i, e in enumerate(events) if e.startswith(prefix)) for prefix in ("git commit ", "git tag ", "git push origin master", "git push origin 3.26.2", "gh release create ")]
            if sequence != sorted(sequence):
                raise AssertionError(f"publication order is wrong: {sequence}")
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

for publish in (False, True):
    try:
        run_case(publish)
    except (AssertionError, StopIteration) as error:
        failed.append(str(error))
if failed:
    raise SystemExit("\n".join(failed))
PY
