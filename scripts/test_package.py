"""Check archive contents, hashes, isolated platforms and repeat packaging."""
import hashlib
import pathlib
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile

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
                for filename in ('README.md', 'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'benchmarks/results.json'):
                    self.assertIn(prefix + filename, names)
                self.assertTrue(all(name == prefix[:-1] or name.startswith(prefix) for name in names))
                digest = hashlib.sha256(archive.read_bytes()).hexdigest()
                self.assertEqual((output / 'SHA256SUMS').read_text(), f'{digest}  {archive.name}\n')
                self.assertEqual(len(list(output.iterdir())), 2, 'Staging files leaked')

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
