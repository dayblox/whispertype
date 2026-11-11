"""Check archive contents, hashes, isolated platforms and repeat packaging."""
import hashlib
import pathlib
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile
import os

SCRIPT = pathlib.Path(__file__).with_name('package.py')


class PackageTests(unittest.TestCase):
    def test_native_packages_and_checksums(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            binary = root / 'fixture'
            binary.write_bytes(b'package fixture')
            for platform in ('windows', 'linux'):
                output = root / platform
                command = [sys.executable, str(SCRIPT), '--platform', platform,
                           f'--{platform}-binary', str(binary), '--output-dir', str(output)]
                for _ in range(2):
                    subprocess.run(command, check=True, capture_output=True)
                archive = next(output.glob('*.zip' if platform == 'windows' else '*.tar.gz'))
                prefix = f'whispertype-{platform}-x64/'
                if platform == 'windows':
                    with zipfile.ZipFile(archive) as bundle:
                        names = bundle.namelist()
                        self.assertEqual(bundle.read(prefix + 'whispertype.exe'), binary.read_bytes())
                        self.assertIn(prefix + 'install-windows.ps1', names)
                        self.assertIn(prefix + 'uninstall-windows.ps1', names)
                else:
                    with tarfile.open(archive) as bundle:
                        names = bundle.getnames()
                        self.assertEqual(bundle.extractfile(prefix + 'whispertype').read(), binary.read_bytes())
                        self.assertEqual(bundle.getmember(prefix + 'whispertype').mode & 0o777, 0o755)
                        self.assertIn(prefix + 'install-linux.sh', names)
                for filename in ('README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'benchmarks/results.json'):
                    self.assertIn(prefix + filename, names)
                self.assertTrue(all(name == prefix[:-1] or name.startswith(prefix) for name in names))
                manifest = (output / 'SHA256SUMS').read_text().splitlines()
                self.assertEqual(len(manifest), 4 if platform == 'windows' else 5)
                for line in manifest:
                    digest, name = line.split('  ', 1)
                    self.assertEqual(hashlib.sha256((output / name).read_bytes()).hexdigest(), digest)
                self.assertFalse(any(path.is_dir() for path in output.iterdir()), 'Staging files leaked')

    @unittest.skipUnless(sys.platform == 'linux', 'Executes a Linux installer with a fake application')
    def test_linux_self_extractor_installs_prebuilt_binary_without_cargo(self):
        with tempfile.TemporaryDirectory(prefix='installer test ') as temp:
            root = pathlib.Path(temp)
            binary = root / 'fixture'
            binary.write_text('#!/bin/sh\nprintf "%s" "$1" > "$HOME/init-called"\n')
            binary.chmod(0o755)
            output = root / 'dist'
            subprocess.run([sys.executable, str(SCRIPT), '--platform', 'linux', '--linux-binary', str(binary),
                            '--output-dir', str(output)], check=True, capture_output=True)
            home = root / 'home'
            home.mkdir()
            env = dict(os.environ, HOME=str(home), XDG_DATA_HOME=str(home / '.local/share'))
            subprocess.run(['bash', str(output / 'whispertype-linux-x64-install.sh')], env=env,
                           check=True, capture_output=True)
            self.assertEqual((home / '.local/bin/whispertype').read_bytes(), binary.read_bytes())
            self.assertEqual((home / 'init-called').read_text(), 'init')
            self.assertTrue((home / '.local/share/whispertype/THIRD_PARTY_NOTICES.txt').is_file())

    def test_missing_binary_fails_before_packaging(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            result = subprocess.run([sys.executable, str(SCRIPT), '--platform', 'windows',
                                     '--windows-binary', str(root / 'missing.exe'),
                                     '--output-dir', str(root / 'dist')], capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((root / 'dist').exists())


if __name__ == '__main__':
    unittest.main()
