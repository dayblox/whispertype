"""Verify both platform manifests and publish a complete GitHub Release."""
import argparse
import hashlib
import pathlib
import re
import shutil
import subprocess
import tempfile
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
REQUIRED = {
    'whispertype-windows-x64.exe', 'whispertype-windows-x64.zip',
    'whispertype-windows-x64-setup.exe', 'whispertype-linux-x64',
    'whispertype-linux-x64.tar.gz', 'whispertype-linux-x64-install.sh',
    'LICENSE', 'THIRD_PARTY_NOTICES.txt',
}


def prepare(artifacts, output):
    files = {}
    manifests = list(artifacts.rglob('SHA256SUMS'))
    if len(manifests) != 2:
        raise ValueError('Expected exactly one checksum manifest for each platform')
    for manifest in manifests:
        for line in manifest.read_text(encoding='utf-8').splitlines():
            digest, name = line.split('  ', 1)
            if name not in REQUIRED or pathlib.Path(name).name != name:
                raise ValueError(f'Unexpected release asset: {name}')
            source = manifest.parent / name
            with source.open('rb') as stream:
                actual = hashlib.file_digest(stream, 'sha256').hexdigest()
            if actual != digest:
                raise ValueError(f'Checksum mismatch: {name}')
            if name in files and files[name][1] != digest:
                raise ValueError(f'Conflicting platform assets: {name}')
            files[name] = source, digest
    if set(files) != REQUIRED:
        raise ValueError(f'Missing release assets: {sorted(REQUIRED - set(files))}')
    output.mkdir(parents=True, exist_ok=False)
    checksums = []
    for name, (source, digest) in sorted(files.items()):
        shutil.copy2(source, output / name)
        checksums.append(f'{digest}  {name}\n')
    (output / 'SHA256SUMS').write_text(''.join(checksums), encoding='utf-8')
    return sorted(output.iterdir())


def publish(files, tag, commit, prerelease=False):
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
    if not re.fullmatch(r'[0-9a-f]{40}', commit):
        raise ValueError('Release target must be a full commit SHA')
    if prerelease:
        if not re.fullmatch(r'main-\d+-[0-9a-f]{7}', tag):
            raise ValueError('Main prerelease tag must be main-RUNNUMBER-SHORTSHA')
    elif tag != f'v{version}':
        raise ValueError(f'Stable tag must match Cargo.toml: v{version}')
    notes = f'''Built from `{commit}` after Windows and Linux tests passed.

**Microphone warm-up fix:** a silent primary still triggers fallback after 300 ms,
but each candidate now gets two seconds from its first native callback for DSP
warm-up. A replacement must open successfully before the old candidate closes;
if none opens, the last candidate keeps buffering until capture ends. The first
live stream wins with its buffered prefix, including a recovered primary. If no
stream wins, finish uses the higher-energy retained buffer. Silent candidates
are never remembered as dead; only dead primary identities are cached in memory.

The fix is generic, with no device-specific rules, new settings, notifications,
idle polling or application threads. Two bounded PCM buffers remain; opening a
replacement briefly overlaps three streams. Loopback/monitor sources are excluded.
The dead-primary cache resets when the default input changes.

The 1.1-second silent-candidate regression, 60 Windows functional tests, release
allocation checks and the native Windows microphone smoke test pass. Windows
resource profiles and their limits are recorded in `benchmarks/windows-mic-warmup.md`.
The reported Intel DSP hardware and native Linux device fallback still need
testing on the destination machine; the delayed-signal case is covered synthetically.

| Platform | Installer | Standalone / portable |
| --- | --- | --- |
| Windows 10/11 x64 | `whispertype-windows-x64-setup.exe` (per user, no admin) | `whispertype-windows-x64.exe` or `whispertype-windows-x64.zip` |
| Linux x64 | `whispertype-linux-x64-install.sh` (self-extracting) | `whispertype-linux-x64` or `whispertype-linux-x64.tar.gz` |

**Windows:** run Setup, choose the installation folder, optionally enter your Groq key, then use Ctrl+Space. The ZIP also includes PowerShell install/uninstall scripts with a `-Destination` option. Standalone users save the key in `%APPDATA%\\whispertype\\groq.key`.

Windows HTTPS uses native WinHTTP/Schannel and the system proxy settings, including machine-wide WinHTTP proxy configuration. Proxy login uses the signed-in Windows account through Negotiate/NTLM, never Basic. Recording warms the Groq connection in the short-lived upload helper; uploads and updates stream through bounded buffers. Linux retains ureq. See `benchmarks/windows-winhttp-efficiency.md` in the source for measured resource costs and validation limits. Corporate proxy login and real-key transcription still need testing in your environment.

**Linux:** run `bash whispertype-linux-x64-install.sh`. It installs the prebuilt binary to `~/.local/bin`, configures the user service/Hyprland bindings, and needs no Rust compiler. Then run `~/.local/bin/whispertype set-key`. The tarball includes `install-linux.sh` as well. For the standalone download, first run `chmod +x whispertype-linux-x64`.

Linux desktop support is currently **Omarchy/Hyprland with Lua bindings on Wayland**, with systemd user services, ALSA/PipeWire, libxkbcommon and wtype. GNOME, X11 and generic non-Hyprland integration are not implemented. The Linux build targets Ubuntu 22.04 (glibc 2.35+) and compatible newer systems; the standalone binary still needs these system libraries.

Verify downloads using `SHA256SUMS`. `LICENSE` and `THIRD_PARTY_NOTICES.txt` apply to the standalone binaries as well. Windows executables are unsigned. No API key is bundled; live Groq transcription and personal wake-word accuracy still require your own validation.
'''
    with tempfile.TemporaryDirectory() as temp:
        note_file = pathlib.Path(temp) / 'notes.md'
        note_file.write_text(notes, encoding='utf-8')
        command = ['gh', 'release', 'create', tag, '--target', commit,
                   '--title', f'Whispertype {tag}', '--notes-file', str(note_file)]
        command += ['--prerelease', '--latest=false'] if prerelease else ['--latest']
        # gh uploads the complete asset set while draft, then publishes it.
        subprocess.run(command + [str(path) for path in files], check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifacts', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--tag')
    parser.add_argument('--commit')
    parser.add_argument('--prerelease', action='store_true')
    args = parser.parse_args()
    files = prepare(args.artifacts, args.output)
    print(f'Verified {len(files)} release assets')
    if args.tag:
        if not args.commit:
            parser.error('--commit is required when publishing')
        publish(files, args.tag, args.commit, args.prerelease)


if __name__ == '__main__':
    main()
