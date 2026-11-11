"""Exercise the real update helper using disposable copies, never the installed app."""
import ctypes as c
from ctypes import wintypes as w
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time


def verify(binary):
    assert sys.platform == 'win32'
    user = c.WinDLL('user32', use_last_error=True)
    callback = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    user.EnumWindows.argtypes = [callback, w.LPARAM]
    user.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
    user.GetClassNameW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
    user.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
    kernel = c.WinDLL('kernel32', use_last_error=True)
    kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
    kernel.OpenProcess.restype = w.HANDLE
    kernel.QueryFullProcessImageNameW.argtypes = [w.HANDLE, w.DWORD, w.LPWSTR, c.POINTER(w.DWORD)]
    kernel.CloseHandle.argtypes = [w.HANDLE]
    children = []

    def windows(executable):
        found = {}

        @callback
        def visit(window, _):
            pid = w.DWORD()
            user.GetWindowThreadProcessId(window, c.byref(pid))
            name = c.create_unicode_buffer(128)
            user.GetClassNameW(window, name, len(name))
            if name.value == 'Whispertype.Native.Tray.v1':
                handle = kernel.OpenProcess(0x1000, False, pid.value)
                if handle:
                    try:
                        path = c.create_unicode_buffer(32768)
                        length = w.DWORD(len(path))
                        if kernel.QueryFullProcessImageNameW(handle, 0, path, c.byref(length)):
                            if Path(path.value).resolve() == executable.resolve():
                                found[pid.value] = window
                    finally:
                        kernel.CloseHandle(handle)
            return True

        user.EnumWindows(visit, 0)
        return found

    with tempfile.TemporaryDirectory(prefix='whispertype-update-test-') as folder:
        root = Path(folder)
        app = root / 'portable français'
        app.mkdir()
        target = app / 'Portable Whispertype.exe'
        shutil.copyfile(binary, target)
        # A harmless PE overlay distinguishes the old executable from its replacement.
        with target.open('ab') as old:
            old.write(b'whispertype-old-version-fixture')
        profile = root / 'profile'
        profile.mkdir()
        config = profile / 'config.toml'
        config.write_text('shortcut = "CTRL + ALT + SHIFT + F24"\nprompt = "Keep my context"\n')
        key = profile / 'groq.key'
        key.write_text('gsk_' + 'x' * 48)
        saved_key = key.read_bytes()
        env = dict(os.environ, WHISPERTYPE_HOME=str(profile))
        env.pop('GROQ_API_KEY', None)

        def start(exe, *args):
            process = subprocess.Popen([str(exe), *map(str, args)], env=env,
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                creationflags=subprocess.CREATE_NO_WINDOW)
            children.append(process)
            return process

        updated = {}
        try:
            parent = start(target, '_worker')
            lingering = start(target, '_worker')
            unrelated = start(binary, '_worker')
            stage = app / '.whispertype-update-integration'
            stage.mkdir()
            shutil.copyfile(binary, stage / 'updater.exe')
            shutil.copyfile(binary, stage / 'new.exe')
            helper = start(stage / 'updater.exe', '_apply-update', target, parent.pid)
            # The updater must wait for the engine, not stop active dictation.
            time.sleep(.4)
            assert helper.poll() is None and parent.poll() is None
            assert not (stage / 'previous.exe').exists()
            parent.communicate(input=b'', timeout=10)
            helper.wait(timeout=20)
            assert helper.returncode == 0, 'Update helper failed'
            lingering.wait(timeout=5)
            assert unrelated.poll() is None, 'A different EXE copy was stopped'
            until = time.monotonic() + 10
            while time.monotonic() < until:
                updated = windows(target)
                if updated and not stage.exists():
                    break
                time.sleep(.05)
            assert len(updated) == 1, 'Updated app did not restart'
            assert not stage.exists(), 'Update helper/staging files were not cleaned up'
            assert hashlib.sha256(target.read_bytes()).digest() == hashlib.sha256(binary.read_bytes()).digest()
            assert key.read_bytes() == saved_key
            assert 'Keep my context' in config.read_text()
            print('Update helper passed: waits for parent exit, stops only matching helpers, '
                  'replaces a portable EXE with spaces/Unicode, restarts, cleans staging, preserves settings/key.')
        finally:
            # Address only this test's restarted app, even if another copy is open.
            updated = windows(target)
            for pid, window in updated.items():
                user.PostMessageW(window, 0x8004, 11, 0)
            until = time.monotonic() + 5
            while windows(target) and time.monotonic() < until:
                time.sleep(.05)
            for process in children:
                if process.poll() is None:
                    process.kill()
                process.communicate(timeout=5)


if __name__ == '__main__':
    verify(Path(sys.argv[1] if len(sys.argv) > 1 else 'target/release/whispertype.exe').resolve(strict=True))
