#!/usr/bin/env python3
"""Release Package.swift generator and checker: target dependencies come from MediaLayer/Package.swift only.

render VERSION CHECKSUM   print the release manifest
check [RELEASE_MANIFEST]  dev and release dependencies match per target, and Swift sources import only declared modules
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEV = ROOT / 'MediaLayer/Package.swift'
SOURCES = ROOT / 'MediaLayer/Sources'
TARGET = re.compile(r'\.target\(\s*name:\s*"(\w+)",\s*dependencies:\s*\[(.*?)\]', re.S)
ITEM = re.compile(r'"(\w+)"|\.product\(\s*name:\s*"(\w+)",\s*package:\s*"(\w+)"\s*\)')


def targets(text):
    """Library targets in order: name -> [(module, literal)]."""
    result = {}
    for name, body in TARGET.findall(text):
        items = [(m.group(1) or m.group(2), m.group(0)) for m in ITEM.finditer(body)]
        result[name] = [(module, re.sub(r'\s+', ' ', literal)) for module, literal in items]
    return result


def render(version, checksum):
    dev = targets(DEV.read_text())
    def target(name):
        deps = ', '.join(literal for _, literal in dev[name])
        return (f'    .target(\n      name: "{name}",\n      dependencies: [{deps}],\n'
                f'      path: "MediaLayer/Sources/{name}",\n      swiftSettings: [.swiftLanguageMode(.v5)]\n    )')
    return f'''// swift-tools-version: 6.0

import PackageDescription

let package = Package(
  name: "MediaLayer",
  platforms: [.iOS(.v17), .macOS(.v14)],
  products: [
    .library(name: "MediaLayer", targets: ["MediaLayer"]),
    .library(name: "MediaLayerUI", targets: ["MediaLayerUI"]),
    .library(name: "MediaLayerTesting", targets: ["MediaLayerTesting"])
  ],
  dependencies: [
    .package(url: "https://github.com/kean/Nuke", from: "13.2.0")
  ],
  targets: [
    .binaryTarget(
      name: "medialayerFFI",
      url: "https://github.com/noma4i/medialayer/releases/download/{version}/MediaLayerFFI.xcframework.zip",
      checksum: "{checksum}"
    ),
{target('MediaLayer')},
{target('MediaLayerUI')},
{target('MediaLayerTesting')}
  ]
)
'''


def check(release_path=None):
    dev = targets(DEV.read_text())
    release_text = Path(release_path).read_text() if release_path else render('0.0.0', '0' * 64)
    release = targets(release_text)
    problems = []
    for name, deps in dev.items():
        if name not in release:
            problems.append(f'release manifest has no target {name}')
        elif [m for m, _ in deps] != [m for m, _ in release[name]]:
            problems.append(f'{name}: dev dependencies {[m for m, _ in deps]} differ from release {[m for m, _ in release[name]]}')
    known = set(dev) | {'medialayerFFI'} | {module for deps in dev.values() for module, _ in deps}
    for name in dev:
        declared = {m for m, _ in release.get(name, [])}
        for file in sorted((SOURCES / name).glob('*.swift')):
            for module in re.findall(r'^\s*(?:@\w+\s+)*import\s+(\w+)', file.read_text(), re.M):
                if module in known and module != name and module not in declared:
                    problems.append(f'{file.relative_to(ROOT)} imports {module} which release target {name} does not depend on')
    return problems


if __name__ == '__main__':
    if len(sys.argv) == 4 and sys.argv[1] == 'render':
        sys.stdout.write(render(sys.argv[2], sys.argv[3]))
    elif sys.argv[1:2] == ['check'] and len(sys.argv) <= 3:
        problems = check(sys.argv[2] if len(sys.argv) == 3 else None)
        print('\n'.join(problems) if problems else 'ok release manifest matches dev manifest and imports')
        sys.exit(1 if problems else 0)
    else:
        sys.exit(__doc__)
