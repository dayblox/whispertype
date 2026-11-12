"""Validate the shipped PE and exercise redirected GUI-subsystem CLI handles."""
import json
import ctypes
from ctypes import wintypes
import os
import pathlib
import struct
import subprocess
import sys
import tempfile


def verify(binary):
    data = binary.read_bytes()
    pe = struct.unpack_from('<I', data, 0x3c)[0]
    assert data[:2] == b'MZ' and data[pe:pe + 4] == b'PE\0\0', 'Invalid Windows executable'
    machine, sections = struct.unpack_from('<HH', data, pe + 4)
    assert machine == 0x8664, 'Expected x64 executable'
    optional = pe + 24
    assert struct.unpack_from('<H', data, optional)[0] == 0x20b, 'Expected PE32+'
    assert struct.unpack_from('<H', data, optional + 68)[0] == 2, 'Must use the GUI subsystem'
    table = optional + struct.unpack_from('<H', data, pe + 20)[0]

    def offset(rva):
        for section in range(sections):
            virtual_size, address, raw_size, raw = struct.unpack_from('<IIII', data, table + section * 40 + 8)
            if address <= rva < address + max(virtual_size, raw_size):
                return raw + rva - address
        raise AssertionError(f'Unmapped PE address {rva:x}')

    imports = []
    resources = offset(struct.unpack_from('<I', data, optional + 128)[0])
    named, numbered = struct.unpack_from('<HH', data, resources + 12)
    resource_types = {struct.unpack_from('<I', data, resources + 16 + index * 8)[0]
                      for index in range(named + numbered)}
    assert {3, 14} <= resource_types, 'Missing embedded icon/group-icon resources'
    extract = ctypes.windll.shell32.ExtractIconExW
    extract.argtypes = [wintypes.LPCWSTR, ctypes.c_int, ctypes.POINTER(wintypes.HICON), ctypes.POINTER(wintypes.HICON), wintypes.UINT]
    extract.restype = wintypes.UINT
    assert extract(str(binary), -1, None, None, 0) > 0, 'Windows cannot extract the application icon'
    descriptor = offset(struct.unpack_from('<I', data, optional + 120)[0])
    while any(data[descriptor:descriptor + 20]):
        name_offset = offset(struct.unpack_from('<I', data, descriptor + 12)[0])
        name = data[name_offset:data.index(b'\0', name_offset)].decode('ascii')
        lowered = name.lower()
        assert not lowered.startswith(('vcruntime', 'msvcp', 'libgcc', 'libstdc++')), f'External runtime: {name}'
        system = pathlib.Path(os.environ['SystemRoot']) / 'System32' / name
        assert system.is_file() or lowered.startswith(('api-ms-win-', 'ext-ms-')), f'Non-system DLL: {name}'
        imports.append(name)
        descriptor += 20
    # WinHTTP is the Windows system HTTP/TLS implementation, not a bundled DLL.
    assert 'winhttp.dll' in {name.lower() for name in imports}, 'Missing native WinHTTP transport'

    with tempfile.TemporaryDirectory(prefix='whispertype-cli-') as temp:
        env = dict(os.environ, WHISPERTYPE_HOME=temp)
        env.pop('GROQ_API_KEY', None)

        def run(*args, input=''):
            return subprocess.run([str(binary), *args], input=input, capture_output=True,
                                  text=True, encoding='utf-8', env=env, timeout=10,
                                  creationflags=subprocess.CREATE_NO_WINDOW)

        help_result = run('--help')
        assert help_result.returncode == 0 and 'Whispertype' in help_result.stdout, help_result.stderr
        config = run('config')
        assert config.returncode == 0 and pathlib.Path(config.stdout.strip()) == pathlib.Path(temp) / 'config.toml'
        invalid = run('invalid-command')
        assert invalid.returncode != 0 and 'Unknown command' in invalid.stderr
        worker = run('_worker')
        result = json.loads(worker.stdout)
        assert worker.returncode == 0 and result['error'] and not result['text']
    print(f'Windows x64 GUI executable verified ({len(data):,} bytes).')
    print('System DLLs: ' + ', '.join(imports))
    print('Embedded application icon, redirected help/config/error and offline worker IPC passed.')


if __name__ == '__main__':
    verify(pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/release/whispertype.exe').resolve())
