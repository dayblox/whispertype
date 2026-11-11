#!/usr/bin/env python3
"""Package native or cross-built executables without rebuilding them."""
import argparse
import hashlib
import pathlib
import shutil
import tarfile
import tempfile
import zipfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
DOCS = ('README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'VALIDATION.md', 'OPTIMIZATION.md')


def package(platform, binary, output):
    if not binary.is_file():
        raise FileNotFoundError(f'Build the {platform} executable first: {binary}')
    output.mkdir(parents=True, exist_ok=True)
    name = f'whispertype-{platform}-x64'
    # Fresh staging avoids shipping deleted files or artifacts from an older build.
    with tempfile.TemporaryDirectory(prefix='package-', dir=output) as temp:
        folder = pathlib.Path(temp) / name
        folder.mkdir()
        for filename in DOCS:
            shutil.copy2(ROOT / filename, folder / filename)
        (folder / 'benchmarks').mkdir()
        for source in sorted((ROOT / 'benchmarks').iterdir()):
            if source.suffix in ('.json', '.html'):
                shutil.copy2(source, folder / 'benchmarks' / source.name)
        shutil.copy2(binary, folder / ('whispertype.exe' if platform == 'windows' else 'whispertype'))
        if platform == 'windows':
            for filename in ('install-windows.ps1', 'uninstall-windows.ps1'):
                shutil.copy2(ROOT / 'scripts' / filename, folder / filename)
            archive = output / f'{name}.zip'
            with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as bundle:
                for path in sorted(folder.rglob('*')):
                    if path.is_file():
                        bundle.write(path, f'{name}/{path.relative_to(folder).as_posix()}')
        else:
            archive = output / f'{name}.tar.gz'
            with tarfile.open(archive, 'w:gz') as bundle:
                bundle.add(folder, arcname=name)
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', choices=('windows', 'linux', 'all'), default='all')
    parser.add_argument('--windows-binary', type=pathlib.Path)
    parser.add_argument('--linux-binary', type=pathlib.Path, default=ROOT / 'target/release/whispertype')
    parser.add_argument('--output-dir', type=pathlib.Path, default=ROOT / 'dist')
    args = parser.parse_args()
    windows = args.windows_binary
    if windows is None:
        windows = next((path for path in (
            ROOT / 'target/release/whispertype.exe',
            ROOT / 'target/x86_64-pc-windows-msvc/release/whispertype.exe',
            ROOT / 'target/x86_64-pc-windows-gnullvm/release/whispertype.exe',
        ) if path.is_file()), ROOT / 'target/release/whispertype.exe')
    targets = {'windows': windows, 'linux': args.linux_binary}
    selected = targets if args.platform == 'all' else {args.platform: targets[args.platform]}
    for platform, binary in selected.items():
        if not binary.is_file():
            parser.error(f'Build the {platform} executable first: {binary}')
    archives = [package(platform, binary, args.output_dir) for platform, binary in selected.items()]
    sums = []
    for path in archives:
        with path.open('rb') as stream:
            sums.append(f'{hashlib.file_digest(stream, "sha256").hexdigest()}  {path.name}\n')
    (args.output_dir / 'SHA256SUMS').write_text(''.join(sums), encoding='utf-8')
    for archive in archives:
        print(archive)


if __name__ == '__main__':
    main()
