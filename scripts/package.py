#!/usr/bin/env python3
"""Package already built Linux and Windows executables without rebuilding them."""
import hashlib
import pathlib
import shutil
import tarfile
import zipfile

root = pathlib.Path(__file__).resolve().parent.parent
dist = root / 'dist'
dist.mkdir(exist_ok=True)
windows = dist / 'whispertype-windows-x64'
linux = dist / 'whispertype-linux-x64'
for folder in (windows, linux):
    folder.mkdir(exist_ok=True)
    for filename in ('README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'VALIDATION.md', 'OPTIMIZATION.md'):
        shutil.copy2(root / filename, folder / filename)
    (folder / 'benchmarks').mkdir(exist_ok=True)
    for filename in ('results.json', 'results.html'):
        shutil.copy2(root / 'benchmarks' / filename, folder / 'benchmarks' / filename)
shutil.copy2(root / 'target/x86_64-pc-windows-gnullvm/release/whispertype.exe', windows)
for name in ('install-windows.ps1', 'uninstall-windows.ps1'):
    shutil.copy2(root / 'scripts' / name, windows)
shutil.copy2(root / 'target/release/whispertype', linux)
with zipfile.ZipFile(dist / 'whispertype-windows-x64.zip', 'w', zipfile.ZIP_DEFLATED) as archive:
    for path in sorted(windows.rglob('*')):
        if path.is_file():
            archive.write(path, f'{windows.name}/{path.relative_to(windows).as_posix()}')
with tarfile.open(dist / 'whispertype-linux-x64.tar.gz', 'w:gz') as archive:
    archive.add(linux, arcname=linux.name)
files = [dist / 'whispertype-windows-x64.zip', dist / 'whispertype-linux-x64.tar.gz']
(dist / 'SHA256SUMS').write_text(''.join(f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n' for p in files))
for path in files:
    print(path)
