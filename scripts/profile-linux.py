#!/usr/bin/env python3
"""Profile an isolated build. Recording/wake modes intentionally open the mic.

The temporary profile has no real API key, and recordings are canceled.
No audio is uploaded. Run while no other app is using the microphone.
"""
import argparse
import json
import os
import pathlib
import socket
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=pathlib.Path, required=True)
parser.add_argument('--mode', choices=['idle', 'record', 'wake'], default='idle')
parser.add_argument('--seconds', type=float, default=10)
parser.add_argument('--output', type=pathlib.Path)
parser.add_argument('--wake-model', type=pathlib.Path, help='Personal wake.rpw required for wake profiling')
args = parser.parse_args()
if not 2 <= args.seconds <= 30:
    parser.error('--seconds must be 2..30')
if args.mode == 'wake' and (args.wake_model is None or not args.wake_model.is_file()):
    parser.error('Wake profiling requires --wake-model pointing to your personal wake.rpw')
env = os.environ.copy()
env.setdefault('XDG_RUNTIME_DIR', f'/run/user/{os.getuid()}')
env.setdefault('DBUS_SESSION_BUS_ADDRESS', f"unix:path={env['XDG_RUNTIME_DIR']}/bus")
env.pop('GROQ_API_KEY', None)
env['WHISPERTYPE_TRACE_AUDIO'] = '1'
desktop = subprocess.check_output(['systemctl', '--user', 'show-environment'], env=env, text=True)
for line in desktop.splitlines():
    if line.startswith(('WAYLAND_DISPLAY=', 'HYPRLAND_INSTANCE_SIGNATURE=', 'XDG_CURRENT_DESKTOP=')):
        key, value = line.split('=', 1)
        env.setdefault(key, value)

def snapshot(pid):
    folder = pathlib.Path(f'/proc/{pid}')
    values = (folder / 'stat').read_text().split()
    status = (folder / 'status').read_text()
    memory = (folder / 'smaps_rollup').read_text()
    result = {
        'cpu_ticks': int(values[13]) + int(values[14]),
        'rss_kib': int(status.split('VmRSS:')[1].split()[0]),
        'pss_kib': int(memory.split('\nPss:')[1].split()[0]),
        'threads': int(status.split('Threads:')[1].split()[0]),
    }
    children = set()
    for task in (folder / 'task').iterdir():
        try:
            children.update(int(value) for value in (task / 'children').read_text().split())
        except FileNotFoundError:
            pass
    result['children'] = len(children)
    result['total_pss_kib'] = result['pss_kib']
    for child in children:
        try:
            result['total_pss_kib'] += snapshot(child)['total_pss_kib']
        except (ProcessLookupError, FileNotFoundError):
            pass
    return result

with tempfile.TemporaryDirectory(prefix='whispertype-profile-') as folder:
    profile = pathlib.Path(folder)
    (profile / 'config.toml').write_text('protect_focus = false\n')
    if args.mode == 'wake':
        (profile / 'wake.rpw').write_bytes(args.wake_model.read_bytes())
    if args.mode == 'record':
        # Only permits opening the microphone. This invalid key is never used:
        # the explicit cancel below prevents submitting the recording.
        (profile / 'groq.key').write_text('gsk_profiling_not_a_valid_api_key')
    env['WHISPERTYPE_HOME'] = folder
    with tempfile.TemporaryFile(mode='w+') as log:
        app = subprocess.Popen([str(args.binary.resolve()), 'run'], env=env, stdout=log, stderr=log)
        control = profile / 'control.sock'
        def command(value):
            with socket.socket(socket.AF_UNIX) as client:
                client.connect(str(control))
                client.sendall(value.encode())
        try:
            for _ in range(100):
                if control.exists():
                    break
                if app.poll() is not None:
                    raise RuntimeError('Application exited during startup')
                time.sleep(0.05)
            time.sleep(0.5)
            if args.mode != 'idle':
                command('start' if args.mode == 'record' else 'wake-on')
            time.sleep(3)
            before = snapshot(app.pid)
            time.sleep(args.seconds)
            after = snapshot(app.pid)
            ticks = after['cpu_ticks'] - before['cpu_ticks']
            report = {
                'binary': str(args.binary), 'mode': args.mode, 'seconds': args.seconds,
                'cpu_tick_delta': ticks,
                'cpu_percent_one_core': 100 * ticks / os.sysconf('SC_CLK_TCK') / args.seconds,
                **{key: value for key, value in after.items() if key != 'cpu_ticks'},
            }
            if args.mode != 'idle':
                command('cancel' if args.mode == 'record' else 'wake-off')
            time.sleep(0.3)
            command('quit')
            app.wait(timeout=5)
            log.seek(0)
            errors = log.read().strip()
            if errors:
                report['log'] = errors
            print(json.dumps(report))
            if args.output:
                args.output.write_text(json.dumps(report, indent=2) + '\n')
        finally:
            if app.poll() is None:
                app.kill()
                app.wait()
