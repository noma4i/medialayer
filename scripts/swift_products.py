#!/usr/bin/env python3
"""Prints the products directory and target triple of the Swift lane build (swift-test step) as two lines."""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PRODUCTS = ROOT / 'target/api-derived-data.noindex/Build/Products/Debug-iphonesimulator'


def products():
    module = PRODUCTS / 'MediaLayer.swiftmodule'
    triples = sorted(path.stem for path in module.glob('*-apple-ios-simulator.swiftmodule'))
    if len(triples) != 1:
        sys.exit(f'swift products: expected one simulator module in {module}, found {triples}')
    package = json.loads(subprocess.check_output(['swift', 'package', 'dump-package'], cwd=ROOT / 'MediaLayer'))
    versions = [platform['version'] for platform in package['platforms'] if platform['platformName'] == 'ios']
    if len(versions) != 1:
        sys.exit('swift products: Package.swift names no single iOS version')
    arch = triples[0].split('-', 1)[0]
    return str(PRODUCTS), f'{arch}-apple-ios{versions[0]}-simulator'


if __name__ == '__main__':
    print('\n'.join(products()))
