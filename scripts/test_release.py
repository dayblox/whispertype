"""Release assembly must fail before publication on missing or corrupt assets."""
import hashlib
import pathlib
import tempfile
import unittest

import release


class ReleaseTests(unittest.TestCase):
    def fixture(self, root):
        for platform in ('windows', 'linux'):
            folder = root / platform
            folder.mkdir()
            lines = []
            for name in sorted(release.REQUIRED):
                if platform not in name and name not in ('LICENSE', 'THIRD_PARTY_NOTICES.txt'):
                    continue
                data = name.encode()
                (folder / name).write_bytes(data)
                lines.append(f'{hashlib.sha256(data).hexdigest()}  {name}\n')
            (folder / 'SHA256SUMS').write_text(''.join(lines))

    def test_complete_release_and_shared_notices(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            incoming = root / 'incoming'
            incoming.mkdir()
            self.fixture(incoming)
            files = release.prepare(incoming, root / 'release')
            self.assertEqual({path.name for path in files}, release.REQUIRED | {'SHA256SUMS'})

    def test_corrupt_or_missing_platform_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = pathlib.Path(temp)
            incoming = root / 'incoming'
            incoming.mkdir()
            self.fixture(incoming)
            (incoming / 'linux/whispertype-linux-x64').write_bytes(b'corrupted')
            with self.assertRaisesRegex(ValueError, 'Checksum mismatch'):
                release.prepare(incoming, root / 'release')
            (incoming / 'linux/SHA256SUMS').unlink()
            with self.assertRaisesRegex(ValueError, 'exactly one'):
                release.prepare(incoming, root / 'release')
            self.assertFalse((root / 'release').exists())


if __name__ == '__main__':
    unittest.main()
