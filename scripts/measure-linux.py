#!/usr/bin/env python3
"""Measure an isolated default tray process. Does not open the microphone."""
import argparse
import json
import os
import pathlib
import socket
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser()
parser.add_argument('--seconds', type=float, default=5)
parser.add_argument('--binary', type=pathlib.Path)
args = parser.parse_args()
if not 1 <= args.seconds <= 60:
    parser.error('--seconds must be between 1 and 60')
root = pathlib.Path(__file__).resolve().parent.parent
env = os.environ.copy()
env.setdefault('XDG_RUNTIME_DIR', f'/run/user/{os.getuid()}')
env.setdefault('DBUS_SESSION_BUS_ADDRESS', f"unix:path={env['XDG_RUNTIME_DIR']}/bus")
desktop_env = subprocess.check_output(['systemctl', '--user', 'show-environment'], env=env, text=True)
for line in desktop_env.splitlines():
    if line.startswith(('WAYLAND_DISPLAY=', 'HYPRLAND_INSTANCE_SIGNATURE=', 'XDG_CURRENT_DESKTOP=')):
        key, value = line.split('=', 1)
        env.setdefault(key, value)

with tempfile.TemporaryDirectory(prefix='whispertype-measure-') as profile:
    env['WHISPERTYPE_HOME'] = profile
    process = subprocess.Popen([str(args.binary or root / 'target/release/whispertype'), 'run'], env=env)
    def snapshot():
        values = pathlib.Path(f'/proc/{process.pid}/stat').read_text().split()
        status = pathlib.Path(f'/proc/{process.pid}/status').read_text()
        return {
            'cpu_ticks': int(values[13]) + int(values[14]),
            'rss_kib': int(status.split('VmRSS:')[1].split()[0]),
            'threads': int(status.split('Threads:')[1].split()[0]),
        }
    try:
        control = pathlib.Path(profile, 'control.sock')
        for _ in range(100):
            if control.exists():
                break
            if process.poll() is not None:
                raise RuntimeError('Whispertype exited before initialization')
            time.sleep(0.05)
        time.sleep(1)
        before = snapshot()
        time.sleep(args.seconds)
        after = snapshot()
        print(json.dumps({
            'sample_seconds': args.seconds,
            'rss_kib': after['rss_kib'],
            'threads': after['threads'],
            'cpu_tick_delta': after['cpu_ticks'] - before['cpu_ticks'],
            'clock_ticks_per_second': os.sysconf('SC_CLK_TCK'),
        }))
        with socket.socket(socket.AF_UNIX) as client:
            client.connect(str(control))
            client.sendall(b'quit')
        process.wait(timeout=5)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
