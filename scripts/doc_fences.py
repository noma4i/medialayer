#!/usr/bin/env python3
import json
import re
import signal
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / 'target/doc-fences'


def uses_compose(body):
    return bool(re.search(r'^\s*[A-Z]\w*\s*\(|\bMedia(?:Image)?Window\s*\(|\bremember\w*\s*\(|\bLaunchedEffect\s*\(|@Composable\b', body, re.M))


def extract():
    documents = [ROOT / 'README.md', *sorted((ROOT / 'docs').glob('*.md'))]
    fences = {'swift': [], 'kotlin': []}
    for document in documents:
        lines = document.read_text().splitlines(keepends=True)
        index = 0
        while index < len(lines):
            match = re.fullmatch(r'```(swift|kotlin)\s*\n?', lines[index])
            if not match:
                index += 1
                continue
            language = match.group(1)
            start = index + 2
            index += 1
            body = []
            while index < len(lines) and not re.fullmatch(r'```\s*\n?', lines[index]):
                body.append(lines[index])
                index += 1
            if index == len(lines):
                raise ValueError(f'{document.relative_to(ROOT)}:{start - 1}: unclosed fence')
            fences[language].append((str(document.relative_to(ROOT)), start, ''.join(body)))
            index += 1
    OUTPUT.mkdir(parents=True, exist_ok=True)
    for language, blocks in fences.items():
        extension = 'swift' if language == 'swift' else 'kt'
        prelude = (ROOT / f'scripts/doc-fences/Prelude.{extension}').read_text()
        if language == 'kotlin':
            imports = ''.join(line for line in prelude.splitlines(keepends=True) if line.startswith('import '))
            generated = [imports + '\n']
        else:
            generated = [prelude.rstrip() + '\n\n']
        mapping = []
        for path, line, body in blocks:
            name = re.sub(r'[^a-zA-Z0-9_]', '_', path)
            declaration = re.match(r'\s*(?:(?:@\w+(?:\([^\n]*\))?\s+)|(?:public|private|internal|final|open|data|sealed|abstract)\s+)*(?:class|struct|enum|protocol|interface|object|actor|typealias|fun)\b', body)
            if language == 'swift':
                if not declaration:
                    generated.append(f'@MainActor func fence_{name}_{line}() async throws {{\n')
                generated.append(f'#sourceLocation(file: "{path}", line: {line})\n')
                generated.append(body)
                generated.append('#sourceLocation()\n')
                if not declaration:
                    generated.append('}\n')
            else:
                if not declaration:
                    kind = '@Composable fun' if uses_compose(body) else 'suspend fun'
                    generated.append(f'{kind} fence_{name}_{line}() {{\n')
                first = sum(part.count('\n') for part in generated) + 1
                generated.append(body)
                mapping.append([first, first + body.count('\n') - 1, path, line])
                if not declaration:
                    generated.append('}\n')
            generated.append('\n')
        (OUTPUT / f'Fences.{extension}').write_text(''.join(generated))
        if language == 'kotlin':
            (OUTPUT / 'kotlin-lines.json').write_text(json.dumps(mapping))


def swift():
    sys.dont_write_bytecode = True
    sys.path.insert(0, str(ROOT / 'scripts'))
    from swift_products import products as built
    products, target = built()
    sdk = subprocess.check_output(['xcrun', '--sdk', 'iphonesimulator', '--show-sdk-path'], text=True).strip()
    return subprocess.call(['xcrun', 'swiftc', '-typecheck', '-parse-as-library', '-target', target, '-sdk', sdk, '-I', products, '-I', str(ROOT / 'target/ios-build/headers/medialayerFFI'), '-F', products, str(OUTPUT / 'Fences.swift')])


def kotlin(command):
    directory = ROOT / 'kotlin/ui/src/test/kotlin'
    directory.mkdir(parents=True, exist_ok=True)
    def terminate(signum, frame):
        raise SystemExit(128 + signum)
    previous = {sig: signal.signal(sig, terminate) for sig in (signal.SIGTERM, signal.SIGINT)}
    try:
        with tempfile.TemporaryDirectory(prefix='docfences-', dir=directory) as source:
            shutil.copyfile(OUTPUT / 'Fences.kt', Path(source) / 'Fences.kt')
            shutil.copyfile(ROOT / 'scripts/doc-fences/Prelude.kt', Path(source) / 'Prelude.kt')
            result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
    mapping = json.loads((OUTPUT / 'kotlin-lines.json').read_text())
    def replace(match):
        number = int(match.group(1))
        for first, last, path, line in mapping:
            if first <= number <= last:
                return f'{path}:{line + number - first}:'
        return match.group(0)
    print(re.sub(r'(?:file://)?[^\s:]*Fences\.kt:(\d+):', replace, result.stdout), end='')
    return result.returncode


if __name__ == '__main__':
    extract()
    if sys.argv[1:] == ['swift']:
        sys.exit(swift())
    if len(sys.argv) > 2 and sys.argv[1] == 'kotlin':
        sys.exit(kotlin(sys.argv[2:]))
    if len(sys.argv) > 1:
        sys.exit('usage: doc_fences.py [swift | kotlin <gradle command...>]')
