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
            if source.suffix in ('.json', '.html', '.md', '.patch'):
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
            (folder / 'install-linux.sh').write_bytes((ROOT / 'scripts/install-linux.sh').read_bytes().replace(b'\r\n', b'\n'))
            (folder / 'install-linux.sh').chmod(0o755)
            (folder / 'whispertype').chmod(0o755)
            archive = output / f'{name}.tar.gz'
            def permissions(info):
                executable = pathlib.PurePosixPath(info.name).name in ('whispertype', 'install-linux.sh')
                info.mode = 0o755 if info.isdir() or executable else 0o644
                return info
            with tarfile.open(archive, 'w:gz') as bundle:
                bundle.add(folder, arcname=name, filter=permissions)
    return archive


def linux_installer(archive, output):
    # A single download containing the prebuilt package. No Rust/Python needed
    # on the destination machine. Extract into a private temporary directory.
    header = b'''#!/bin/sh
set -eu
if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
  echo 'This installer requires Linux x86_64.' >&2
  exit 1
fi
command -v bash >/dev/null || { echo 'Please install bash.' >&2; exit 1; }
work_dir=$(mktemp -d "${TMPDIR:-/tmp}/whispertype-install.XXXXXXXX")
trap 'rm -rf -- "$work_dir"' EXIT HUP INT TERM
payload_line=$(awk '/^__WHISPERTYPE_PAYLOAD__$/ { print NR + 1; exit }' "$0")
tail -n +"$payload_line" "$0" | tar -xz -C "$work_dir"
bash "$work_dir/whispertype-linux-x64/install-linux.sh"
exit 0
__WHISPERTYPE_PAYLOAD__
'''
    path = output / 'whispertype-linux-x64-install.sh'
    with path.open('wb') as stream:
        stream.write(header)
        with archive.open('rb') as payload:
            shutil.copyfileobj(payload, stream)
    path.chmod(0o755)
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', choices=('windows', 'linux', 'all'), default='all')
    parser.add_argument('--windows-binary', type=pathlib.Path)
    parser.add_argument('--windows-installer', type=pathlib.Path)
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
    if args.windows_installer and not args.windows_installer.is_file():
        parser.error(f'Windows installer not found: {args.windows_installer}')
    archives = []
    for platform, binary in selected.items():
        archive = package(platform, binary, args.output_dir)
        archives.append(archive)
        standalone = args.output_dir / ('whispertype-windows-x64.exe' if platform == 'windows' else 'whispertype-linux-x64')
        shutil.copy2(binary, standalone)
        if platform == 'linux':
            standalone.chmod(0o755)
            archives.append(linux_installer(archive, args.output_dir))
        elif args.windows_installer:
            installer = args.output_dir / 'whispertype-windows-x64-setup.exe'
            if args.windows_installer.resolve() != installer.resolve():
                shutil.copy2(args.windows_installer, installer)
            archives.append(installer)
        archives.append(standalone)
    for name in ('LICENSE', 'THIRD_PARTY_NOTICES.txt'):
        notice = args.output_dir / name
        # Shared release assets must have identical hashes on CRLF Windows and
        # LF Linux checkouts. Archives retain their platform's documentation.
        notice.write_bytes((ROOT / name).read_bytes().replace(b'\r\n', b'\n'))
        archives.append(notice)
    sums = []
    for path in archives:
        with path.open('rb') as stream:
            sums.append(f'{hashlib.file_digest(stream, "sha256").hexdigest()}  {path.name}\n')
    (args.output_dir / 'SHA256SUMS').write_text(''.join(sums), encoding='utf-8')
    for archive in archives:
        print(archive)


if __name__ == '__main__':
    main()
