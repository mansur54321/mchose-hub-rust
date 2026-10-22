#!/usr/bin/env python3
"""Package a built application together with its firmware and notices."""

import argparse
import hashlib
import json
from pathlib import Path
import tarfile
import zipfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', required=True, choices=['linux-x86_64', 'windows-x86_64'])
    parser.add_argument('--binary', required=True, type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f'binary not found: {binary}')
    firmware = root / 'firmware'
    for entry in json.loads((firmware / 'manifest.json').read_text()):
        data = (firmware / entry['filename']).read_bytes()
        if len(data) != entry['bytes'] or hashlib.sha256(data).hexdigest() != entry['sha256']:
            raise ValueError(f"Firmware manifest mismatch: {entry['filename']}")
    files = [(binary, binary.name)]
    for name in ['README.md', 'README.ru.md', 'NOTICES.md']:
        files.append((root / name, name))
    for folder in ['firmware', 'docs']:
        files.extend((path, path.relative_to(root).as_posix()) for path in sorted((root / folder).rglob('*')) if path.is_file())
    files.append((root / 'assets/provenance.json', 'assets/provenance.json'))
    for name in ['LICENSE-MIT', 'LICENSE-APACHE', 'MCHOSE-PATCH.md']:
        path = root / 'vendor/egui-winit' / name
        files.append((path, path.relative_to(root).as_posix()))
    if args.platform.startswith('windows'):
        files.extend([
            (root / 'scripts/install-windows.ps1', 'install-windows.ps1'),
            (root / 'assets/mchose-icon.ico', 'mchose-icon.ico'),
        ])
    else:
        files.extend([
            (root / 'scripts/install-local.py', 'scripts/install-local.py'),
            (root / 'assets/mchose-icon.png', 'assets/mchose-icon.png'),
        ])
    output = root / 'dist'
    output.mkdir(exist_ok=True)
    name = f'mchose-native-{args.platform}'
    if args.platform.startswith('windows'):
        archive = output / f'{name}.zip'
        with zipfile.ZipFile(archive, 'x', compression=zipfile.ZIP_DEFLATED) as package:
            for path, relative in files:
                package.write(path, f'{name}/{relative}')
    else:
        archive = output / f'{name}.tar.gz'
        with tarfile.open(archive, 'x:gz') as package:
            for path, relative in files:
                package.add(path, arcname=f'{name}/{relative}')
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksum = archive.with_name(archive.name + '.sha256')
    with checksum.open('x') as handle:
        handle.write(f'{digest}  {archive.name}\n')
    print(archive)
    print(f'SHA256 {digest}')


if __name__ == '__main__':
    main()
