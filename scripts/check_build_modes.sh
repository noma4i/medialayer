#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
source "$root/scripts/build_common.sh"

artifact_cases() {
  local kind expected filenames out code failed=0
  while IFS=' ' read -r kind expected filenames; do
    code=0
    out="$(printf '{"reason":"compiler-artifact","target":{"name":"medialayer","kind":["lib","cdylib","staticlib"]},"filenames":[%s]}' "$filenames" | reported_artifact "$kind" medialayer 2>/dev/null)" || code=$?
    if { [ "$expected" = FAIL ] && [ "$code" -ne 0 ] && [ -z "$out" ]; } ||
       { [ "$expected" != FAIL ] && [ "$code" -eq 0 ] && [ "$out" = "$expected" ]; }; then
      echo "ok $kind artifact $expected"
    else
      echo "FAIL $kind artifact: rc=$code out=[$out] expected=[$expected]"
      failed=1
    fi
  done <<'CASES'
staticlib /t/libmedialayer.a "/t/libmedialayer.rlib","/t/libmedialayer.dylib","/t/libmedialayer.a"
staticlib /t/libmedialayer.a "/t/libmedialayer.a","/t/libmedialayer.rlib","/t/libmedialayer.dylib"
cdylib /t/libmedialayer.dylib "/t/libmedialayer.rlib","/t/libmedialayer.dylib","/t/libmedialayer.a"
cdylib /t/libmedialayer.so "/t/libmedialayer.a","/t/libmedialayer.rlib","/t/libmedialayer.so"
staticlib FAIL "/t/libmedialayer.rlib","/t/libmedialayer.dylib"
cdylib FAIL "/t/libmedialayer.rlib","/t/libmedialayer.a"
CASES
  return "$failed"
}
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
mode_name = sys.argv[2]
cleanup = mode_name == "cleanup"
stub = '''#!/usr/bin/env python3
import json, os, pathlib, sys
p = pathlib.Path
root = p(os.environ["BUILD_TEST_ROOT"])
tool = p(sys.argv[0]).name
base = p(os.environ.get("CARGO_TARGET_DIR", str(root / "target")))
args = sys.argv[1:]
with (root / "calls").open("a") as log:
    log.write(json.dumps([tool, os.getcwd(), *args]) + "\\n")
def write(name, data="artifact"):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(data)
if tool == "uname": print(os.environ["BUILD_TEST_OS"])
elif tool in ("nproc", "sysctl"): sys.exit(9)
elif tool == "getconf": print(2)
elif tool == "cargo":
    with (root / "jobs").open("a") as log:
        log.write(os.environ.get("CARGO_BUILD_JOBS", "") + " " + os.environ.get("RUST_TEST_THREADS", "") + "\\n")
    if args[0] == "ndk":
        output = p(args[args.index("-o") + 1])
        for i, arg in enumerate(args):
            if arg == "-t": write(output / args[i + 1] / "libmedialayer.so")
    elif "--target" in args:
        artifact = base / args[args.index("--target") + 1] / "release/libmedialayer.a"
        write(artifact)
        if "--message-format=json-render-diagnostics" in args:
            print(json.dumps({"reason": "compiler-artifact", "target": {"name": "other", "kind": ["staticlib"]}, "filenames": [str(base / "other.a")]}))
            print(json.dumps({"reason": "compiler-artifact", "target": {"name": "medialayer", "kind": ["lib", "cdylib", "staticlib"]}, "filenames": [str(base / "libmedialayer.rlib"), str(artifact)]}))
    else:
        bindgen = (base / "debug") if "CARGO_TARGET_DIR" in os.environ else (root / os.environ["BUILD_TEST_LIB"]).parent
        bindgen = bindgen / "uniffi-bindgen"
        write(bindgen, p(sys.argv[0]).read_text())
        bindgen.chmod(0o755)
        if "--bin" in args and "--message-format=json-render-diagnostics" in args:
            print(json.dumps({"reason": "compiler-artifact", "package_id": "uniffi-bindgen 0.1.0", "target": {"name": "uniffi-bindgen", "kind": ["bin"]}, "executable": str(bindgen)}))
        if "--bin" not in args:
            library = os.environ["BUILD_TEST_LIB"]
            write(library)
            if "--message-format=json-render-diagnostics" in args:
                print(json.dumps({"reason": "compiler-artifact", "package_id": "other 0.1.0", "target": {"name": "other", "kind": ["lib"]}, "filenames": [str(root / "target/debug/libother.rlib")]}))
                print(json.dumps({"reason": "compiler-artifact", "package_id": "medialayer 0.1.0", "target": {"name": "medialayer", "kind": ["cdylib", "lib"]}, "filenames": [str(root / library)]}))
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
    "--check": ["aarch64-apple-ios-sim"],
    "--ios": ["aarch64-apple-ios", "aarch64-apple-ios-sim"],
    "": ["aarch64-apple-ios", "aarch64-apple-ios-sim"],
}
android = {
    "--release": ["arm64-v8a", "armeabi-v7a"],
    "--abis": ["arm64-v8a", "armeabi-v7a", "x86_64", "x86"],
    "--host": [],
    "": ["arm64-v8a", "armeabi-v7a", "x86_64", "x86"],
}
failed = []
cases = {
    "cleanup": [("kotlin", {"--release": android["--release"]}, "Linux", None, None)],
    "targetdir": [("ios", {"--ios": ios["--ios"]}, "Darwin", "elsewhere", None)],
    "targets": [
        ("ios", ios, "Darwin", None, None),
        ("kotlin", android, "Linux", None, None),
        ("kotlin", {"--host": []}, "Darwin", None, None),
        ("kotlin", {"--host": []}, "Darwin", None, "target/aarch64-apple-darwin/debug/libmedialayer.dylib"),
        ("kotlin", {"--host": []}, "MINGW64_NT", None, "target/x86_64-pc-windows-msvc/debug/medialayer.dll"),
    ],
}[mode_name]
for platform, modes, host_os, target_dir, custom_library in cases:
    for mode, targets in modes.items():
        library = custom_library or "target/debug/libmedialayer." + ("dylib" if host_os == "Darwin" else "so")
        label = f"{platform} {mode or 'default'} {host_os} {library}" + (f" target dir {target_dir}" if target_dir else "")
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory).resolve()
            shutil.copytree(root / "scripts", work / "scripts")
            script = work / "scripts" / f"build_{platform}.sh"
            script.write_text(script.read_text().replace('export PATH="/opt/homebrew/opt/rustup/bin:$PATH"', ""))
            (work / "scripts/swift_cancellation.py").write_text("")
            (work / "core").mkdir()
            (work / "bin").mkdir()
            (work / "kotlin").mkdir()
            for tool in ["cargo", "uname", "sysctl", "nproc", "getconf", "xcodebuild"]:
                path = work / "bin" / tool
                path.write_text(stub)
                path.chmod(0o755)
            jni = work / "kotlin/src/main/jniLibs"
            if cleanup:
                for abi in android["--abis"]:
                    (jni / abi).mkdir(parents=True)
                    (jni / abi / "libmedialayer.so").write_text("stale")
            env = dict(os.environ, PATH=f"{work}/bin:{os.environ['PATH']}",
                       BUILD_TEST_ROOT=str(work), BUILD_TEST_OS=host_os, BUILD_TEST_LIB=library,
                       ANDROID_NDK_HOME=str(work))
            env.pop("CARGO_TARGET_DIR", None)
            if target_dir: env["CARGO_TARGET_DIR"] = str(work / target_dir)
            env.pop("CARGO_BUILD_JOBS", None)
            env.pop("RUST_TEST_THREADS", None)
            result = subprocess.run(["bash", str(script), *([mode] if mode else [])], cwd=work, env=env, capture_output=True, text=True)
            try:
                assert result.returncode == 0, result.stderr
                if cleanup:
                    assert sorted(path.name for path in jni.iterdir()) == sorted(targets)
                    assert all((jni / abi / "libmedialayer.so").read_text() == "artifact" for abi in targets)
                    print("ok release replaces ARM libraries and removes x86 libraries")
                    continue
                calls = [json.loads(line) for line in (work / "calls").read_text().splitlines()]
                assert not any(call[0] in ("sysctl", "nproc") for call in calls), calls
                assert set((work / "jobs").read_text().splitlines()) == {"2 2"}, (work / "jobs").read_text()
                builds = [call[2:] for call in calls if call[0] == "cargo"]
                generators = [call[2:] for call in calls if call[0] == "uniffi-bindgen"]
                if platform == "ios":
                    assert all(call[1] == str(work / "core") for call in calls if call[0] == "uniffi-bindgen"), calls
                    actual = [call[call.index("--target") + 1] for call in builds if "--target" in call]
                    assert actual == targets, actual
                    assert all("--release" in call for call in builds if "--target" in call)
                    assert all(call[call.index("--manifest-path") + 1] == str(work / "core/Cargo.toml") for call in builds if "--target" in call), builds
                    host = [call for call in builds if "--target" not in call]
                    assert len(host) == 1 and "--bin" in host[0], host
                    assert str(work / "bindgen/Cargo.toml") in host[0], host
                    assert len(generators) == 1 and generators[0][generators[0].index("--library") + 1].endswith(f"/{targets[0]}/release/libmedialayer.a"), generators
                    packaging = [call[2:] for call in calls if call[0] == "xcodebuild"]
                    libraries = [packaging[0][i + 1] for i, arg in enumerate(packaging[0]) if arg == "-library"]
                    built = work / (target_dir or "target")
                    assert libraries == [str(built / f"{target}/release/libmedialayer.a") for target in targets], libraries
                    assert generators[0][generators[0].index("--library") + 1] == libraries[0], generators
                else:
                    assert all(call[1] == str(work / "core") for call in calls if call[0] == "uniffi-bindgen" or call[0] == "cargo" and len(call) > 2 and call[2] == "ndk"), calls
                    core_builds = [call for call in builds if call[0] != "ndk" and "--bin" not in call]
                    assert all(call[call.index("--manifest-path") + 1] == str(work / "core/Cargo.toml") for call in core_builds), core_builds
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
                        assert (work / "kotlin/host-library.path").read_text().strip() == str(work / library).rsplit("/", 1)[0], (work / "kotlin/host-library.path").read_text()
                        assert generators[0][generators[0].index("--library") + 1] == str(work / library), generators
                print(f"ok {label}")
            except (AssertionError, IndexError) as error:
                failed.append(label)
                print(f"FAIL {label}: {error}")
sys.exit(bool(failed))
PY
}

targets_with_ambient_target_dir() {
  local outer file status=0
  outer="$(mktemp -d)"
  for file in aarch64-apple-ios/release/libmedialayer.a aarch64-apple-ios-sim/release/libmedialayer.a debug/uniffi-bindgen; do
    mkdir -p "$outer/$(dirname "$file")"
    printf sentinel > "$outer/$file"
  done
  CARGO_TARGET_DIR="$outer" check_modes targets || status=$?
  for file in aarch64-apple-ios/release/libmedialayer.a aarch64-apple-ios-sim/release/libmedialayer.a debug/uniffi-bindgen; do
    [ "$(cat "$outer/$file")" = sentinel ] || { echo "FAIL ambient CARGO_TARGET_DIR overwritten: $file"; status=1; }
  done
  rm -rf "$outer"
  return "$status"
}

inv_build_01_selected_targets_and_bindings() {
  expect_green targets_with_ambient_target_dir
}

inv_build_02_release_removes_stale_x86_libraries() {
  expect_green check_modes cleanup
}

bounded_step_jobs() {
  local work out
  work="$(mktemp -d)"
  mkdir "$work/bin"
  printf '#!/bin/sh\nexit 9\n' > "$work/bin/sysctl"
  cp "$work/bin/sysctl" "$work/bin/nproc"
  printf '#!/bin/sh\n[ "$1" = _NPROCESSORS_ONLN ] && echo 2\n' > "$work/bin/getconf"
  chmod +x "$work/bin"/*
  out="$(env -u CARGO_BUILD_JOBS -u RUST_TEST_THREADS PATH="$work/bin:$PATH" bash "$root/scripts/check_bounded.sh" 10 jobs sh -c 'echo "$CARGO_BUILD_JOBS $RUST_TEST_THREADS"')"
  rm -rf "$work"
  [ "$out" = "2 2" ] || { echo "FAIL bounded step jobs: [$out]"; return 1; }
  echo "ok bounded step jobs"
}

inv_build_03_bounded_step_gets_the_cpu_count_from_one_portable_call() {
  expect_green bounded_step_jobs
}

inv_build_04_ios_artifacts_follow_the_cargo_target_dir() {
  expect_green check_modes targetdir
}

artifact_cases
inv_build_01_selected_targets_and_bindings
inv_build_02_release_removes_stale_x86_libraries
inv_build_04_ios_artifacts_follow_the_cargo_target_dir
inv_build_03_bounded_step_gets_the_cpu_count_from_one_portable_call
