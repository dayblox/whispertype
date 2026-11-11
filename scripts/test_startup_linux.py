"""Exercise startup toggles through the real headless app and a fake systemctl."""
import os
import pathlib
import socket
import subprocess
import sys
import tempfile
import time
import unittest


@unittest.skipUnless(sys.platform == 'linux', 'Linux-only isolated startup integration')
class StartupTests(unittest.TestCase):
    def test_toggle_persists_without_starting_or_stopping_session(self):
        binary = pathlib.Path('target/release/whispertype').resolve()
        with tempfile.TemporaryDirectory(prefix='whispertype-startup-') as temp:
            root = pathlib.Path(temp)
            profile = root / 'profile'
            profile.mkdir()
            (profile / 'config.toml').write_text('tray = false\nindicator = false\nwake_enabled = true\n')
            tools = root / 'bin'
            tools.mkdir()
            controller = tools / 'systemctl'
            controller.write_text('''#!/bin/sh
printf '%s\\n' "$*" >> "$STARTUP_TEST_LOG"
case "$*" in
  '--user is-enabled whispertype.service')
    if [ -f "$STARTUP_TEST_STATE" ]; then echo enabled; exit 0; else echo disabled; exit 1; fi ;;
  '--user daemon-reload') exit 0 ;;
  '--user enable whispertype.service') touch "$STARTUP_TEST_STATE" ;;
  '--user disable whispertype.service') rm -f "$STARTUP_TEST_STATE" ;;
  *) echo 'Unexpected service operation' >&2; exit 99 ;;
esac
''')
            controller.chmod(0o755)
            notifier = tools / 'notify-send'
            notifier.write_text('''#!/bin/sh
for argument do last="$argument"; done
printf '%s\\n' "$last" >> "$FEEDBACK_TEST_LOG"
echo 42
''')
            notifier.chmod(0o755)
            state = root / 'enabled'
            log = root / 'commands.log'
            env = dict(os.environ, HOME=str(root / 'home'), XDG_CONFIG_HOME=str(root / 'config'),
                       WHISPERTYPE_HOME=str(profile), PATH=f'{tools}:{os.environ["PATH"]}',
                       STARTUP_TEST_STATE=str(state), STARTUP_TEST_LOG=str(log))
            env['FEEDBACK_TEST_LOG'] = str(root / 'feedback.log')
            app = subprocess.Popen([str(binary), 'run'], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

            def wait_for(predicate):
                deadline = time.monotonic() + 5
                while not predicate():
                    self.assertIsNone(app.poll(), 'App exited unexpectedly')
                    if time.monotonic() >= deadline:
                        self.fail('Startup operation timed out')
                    time.sleep(0.02)

            def send(command):
                with socket.socket(socket.AF_UNIX) as client:
                    client.connect(str(profile / 'control.sock'))
                    client.sendall(command.encode())

            try:
                wait_for(lambda: (profile / 'control.sock').exists())
                self.assertIn('wake_enabled = false', (profile / 'config.toml').read_text())
                send('startup-toggle')
                wait_for(state.exists)
                unit = root / 'config/systemd/user/whispertype.service'
                self.assertIn(f'ExecStart="{binary}" run', unit.read_text())
                # Allow the event loop to consume its successful startup result.
                time.sleep(0.1)
                send('startup-toggle')
                wait_for(lambda: not state.exists())
                self.assertIsNone(app.poll(), 'Disabling startup stopped the current session')
                commands = log.read_text().splitlines()
                self.assertIn('--user enable whispertype.service', commands)
                self.assertIn('--user disable whispertype.service', commands)
                self.assertFalse(any('--now' in line or ' start ' in line or ' stop ' in line for line in commands))
                send('wake-on')
                feedback = root / 'feedback.log'
                wait_for(lambda: feedback.exists() and 'Click Record when you are ready' in feedback.read_text())
                self.assertFalse((profile / 'wake.rpw').exists(), 'Wake must require personal enrollment')
                send('enroll-cancel')
                send('quit')
                stdout, stderr = app.communicate(timeout=5)
                self.assertEqual(app.returncode, 0, stderr.decode())
            finally:
                if app.poll() is None:
                    app.kill()
                app.communicate(timeout=5)


if __name__ == '__main__':
    unittest.main()
