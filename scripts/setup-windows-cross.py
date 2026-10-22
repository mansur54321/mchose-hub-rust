#!/usr/bin/env python3
"""Install pinned cross-build tools inside this checkout; no global configuration."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
TOOLS = ROOT / '.tools'
MANIFEST = json.loads((ROOT / 'scripts/windows-toolchains.json').read_text())


def download(source):
    destination = TOOLS / 'downloads' / source['url'].rsplit('/', 1)[1]
    destination.parent.mkdir(parents=True, exist_ok=True)
    if not destination.exists():
        print(f"Downloading {destination.name}", flush=True)
        partial = destination.with_suffix(destination.suffix + '.partial')
        with urllib.request.urlopen(source['url'], timeout=120) as response, partial.open('wb') as output:
            shutil.copyfileobj(response, output)
        os.replace(partial, destination)
    with destination.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    if digest != source['sha256']:
        raise RuntimeError(f"SHA-256 mismatch: {destination}; preserve it for inspection and retry with a verified archive")
    return destination


def merge_tree(source, destination):
    def existing_links(directory, names):
        ignored = []
        for name in names:
            origin = Path(directory) / name
            target = destination / origin.relative_to(source)
            if origin.is_symlink() and target.is_symlink():
                if origin.readlink() != target.readlink():
                    raise RuntimeError(f"Refusing to replace a different symlink: {target}")
                ignored.append(name)
        return ignored
    shutil.copytree(source, destination, symlinks=True, dirs_exist_ok=True, ignore=existing_links)


def install(source, archive):
    marker = TOOLS / ('.installed-' + archive.name.removesuffix('.tar.xz'))
    if marker.exists() and marker.read_text().strip() == source['sha256']:
        return
    print(f"Extracting {archive.name}", flush=True)
    with tempfile.TemporaryDirectory(prefix='extract-', dir=TOOLS) as temporary:
        with tarfile.open(archive) as bundle:
            bundle.extractall(temporary, filter='data')
        package = next(Path(temporary).iterdir())
        if source['kind'] == 'llvm-mingw':
            destination = TOOLS / package.name
            merge_tree(package, destination)
            link = TOOLS / 'llvm-mingw'
            if not link.exists():
                link.symlink_to(destination.name, target_is_directory=True)
            elif link.resolve() != destination.resolve():
                raise RuntimeError(f"Refusing to replace another toolchain: {link}")
        else:
            components = (package / 'components').read_text().splitlines()
            for component in components:
                merge_tree(package / component, TOOLS / 'rust-sysroot')
    marker.write_text(source['sha256'] + '\n')


if __name__ == '__main__':
    TOOLS.mkdir(exist_ok=True)
    for source in MANIFEST['sources']:
        install(source, download(source))
    print('Ready: scripts/build-windows.sh')
