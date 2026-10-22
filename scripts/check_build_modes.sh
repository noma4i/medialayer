#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
expect_green() {
  "$@" || return 1
}

check_modes() {
python3 - "$root" "$1" <<'PY'
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(sys.argv[1])
cleanup = sys.argv[2] == "cleanup"
stub = '''#!/usr/bin/env python3
import json, os, pathlib, sys
p = pathlib.Path
root = p(os.environ["BUILD_TEST_ROOT"])
tool = p(sys.argv[0]).name
args = sys.argv[1:]
with (root / "calls").open("a") as log:
    log.write(json.dumps([tool, *args]) + "\\n")
def write(name, data="artifact"):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(data)
if tool == "uname": print(os.environ["BUILD_TEST_OS"])
elif tool == "nproc": print(2)
elif tool == "sysctl":
    if os.environ["BUILD_TEST_OS"] != "Darwin": sys.exit(9)
    print(2)
elif tool == "cargo":
    if args[0] == "ndk":
        output = p(args[args.index("-o") + 1])
        for i, arg in enumerate(args):
            if arg == "-t": write(output / args[i + 1] / "libmedialayer.so")
    elif "--target" in args:
        write("target/" + args[args.index("--target") + 1] + "/release/libmedialayer.a")
    else:
        write("target/debug/uniffi-bindgen", p(sys.argv[0]).read_text())
        (root / "target/debug/uniffi-bindgen").chmod(0o755)
        if "--bin" not in args: write("target/debug/libmedialayer." + ("dylib" if os.environ["BUILD_TEST_OS"] == "Darwin" else "so"))
elif tool == "uniffi-bindgen":
    library = p(args[args.index("--library") + 1])
    if not library.is_file(): sys.exit(8)
    out = p(args[args.index("--out-dir") + 1])
    if args[args.index("--language") + 1] == "swift":
        for name in ["medialayer.swift", "medialayerFFI.h", "medialayerFFI.modulemap"]: write(out / name)
    else: write(out / "com/noma4i/medialayer/medialayer.kt")
elif tool == "xcodebuild":
    for i, arg in enumerate(args):
        if arg == "-library" and not p(args[i + 1]).is_file(): sys.exit(7)
    p(args[args.index("-output") + 1]).mkdir()
'''

import json
ios = {
    "--device": ["aarch64-apple-ios"],
    "--check": ["aarch64-apple-darwin", "aarch64-apple-ios-sim"],
    "--ios": ["aarch64-apple-ios", "aarch64-apple-ios-sim"],
    "": ["aarch64-apple-darwin", "aarch64-apple-ios", "aarch64-apple-ios-sim"],
}
android = {
    "--release": ["arm64-v8a", "armeabi-v7a"],
    "--abis": ["arm64-v8a", "armeabi-v7a", "x86_64", "x86"],
    "--host": [],
    "": ["arm64-v8a", "armeabi-v7a", "x86_64", "x86"],
}
failed = []
cases = [("kotlin", {"--release": android["--release"]}, "Linux")] if cleanup else [("ios", ios, "Darwin"), ("kotlin", android, "Linux"), ("kotlin", {"--host": []}, "Darwin")]
for platform, modes, host_os in cases:
    for mode, targets in modes.items():
        label = f"{platform} {mode or 'default'} {host_os}"
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory).resolve()
            shutil.copytree(root / "scripts", work / "scripts")
            script = work / "scripts" / f"build_{platform}.sh"
            script.write_text(script.read_text().replace('export PATH="/opt/homebrew/opt/rustup/bin:$PATH"', ""))
            (work / "scripts/swift_cancellation.py").write_text("")
            (work / "core").mkdir()
            (work / "bin").mkdir()
            for tool in ["cargo", "uname", "sysctl", "nproc", "xcodebuild"]:
                path = work / "bin" / tool
                path.write_text(stub)
                path.chmod(0o755)
            jni = work / "kotlin/src/main/jniLibs"
            if cleanup:
                for abi in android["--abis"]:
                    (jni / abi).mkdir(parents=True)
                    (jni / abi / "libmedialayer.so").write_text("stale")
            env = dict(os.environ, PATH=f"{work}/bin:{os.environ['PATH']}",
                       BUILD_TEST_ROOT=str(work), BUILD_TEST_OS=host_os,
                       ANDROID_NDK_HOME=str(work))
            env.pop("CARGO_BUILD_JOBS", None)
            env.pop("RUST_TEST_THREADS", None)
            result = subprocess.run(["bash", str(script), *([mode] if mode else [])], env=env, capture_output=True, text=True)
            try:
                assert result.returncode == 0, result.stderr
                if cleanup:
                    assert sorted(path.name for path in jni.iterdir()) == sorted(targets)
                    assert all((jni / abi / "libmedialayer.so").read_text() == "artifact" for abi in targets)
                    print("ok release replaces ARM libraries and removes x86 libraries")
                    continue
                calls = [json.loads(line) for line in (work / "calls").read_text().splitlines()]
                builds = [call[1:] for call in calls if call[0] == "cargo"]
                generators = [call for call in calls if call[0] == "uniffi-bindgen"]
                if platform == "ios":
                    actual = [call[call.index("--target") + 1] for call in builds if "--target" in call]
                    assert actual == targets, actual
                    assert all("--release" in call for call in builds if "--target" in call)
                    host = [call for call in builds if "--target" not in call]
                    assert len(host) == 1 and "--bin" in host[0], host
                    assert str(work / "bindgen/Cargo.toml") in host[0], host
                    assert len(generators) == 1 and generators[0][generators[0].index("--library") + 1].endswith(f"/{targets[0]}/release/libmedialayer.a"), generators
                    packaging = [call for call in calls if call[0] == "xcodebuild"]
                    libraries = [packaging[0][i + 1] for i, arg in enumerate(packaging[0]) if arg == "-library"]
                    assert libraries == [str(work / f"target/{target}/release/libmedialayer.a") for target in targets], libraries
                else:
                    assert not any(call[0] == "sysctl" for call in calls), calls
                    assert any(call[0] == "nproc" for call in calls), calls
                    ndk = [call for call in builds if call[0] == "ndk"]
                    actual = [call[i + 1] for call in ndk for i, arg in enumerate(call) if arg == "-t"]
                    assert actual == targets, actual
                    host = [call for call in builds if call[0] != "ndk"]
                    assert len(host) == (0 if mode == "--abis" else 1 if mode == "--release" else 2), host
                    assert len(generators) == (0 if mode == "--abis" else 1), generators
                    if mode == "--release":
                        assert "--bin" in host[0], host
                        assert str(work / "bindgen/Cargo.toml") in host[0], host
                        assert generators[0][generators[0].index("--library") + 1] == str(jni / "arm64-v8a/libmedialayer.so"), generators
                    elif generators:
                        assert generators[0][generators[0].index("--library") + 1].endswith("/debug/libmedialayer." + ("dylib" if host_os == "Darwin" else "so")), generators
                print(f"ok {label}")
            except (AssertionError, IndexError) as error:
                failed.append(label)
                print(f"FAIL {label}: {error}")
sys.exit(bool(failed))
PY
}

inv_build_01_selected_targets_and_bindings() {
  expect_green check_modes targets
}

inv_build_02_release_removes_stale_x86_libraries() {
  expect_green check_modes cleanup
}

inv_build_01_selected_targets_and_bindings
inv_build_02_release_removes_stale_x86_libraries
