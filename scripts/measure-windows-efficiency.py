"""Repeatable app-only Windows profiling; isolated profile, uploads disabled.

Controls are addressed by PID, so another installed Whispertype is untouched.
Do not compile or run other benchmarks during a measurement.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import hashlib
import json
import os
import pathlib
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import winreg

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=pathlib.Path, required=True)
parser.add_argument('--label', required=True)
parser.add_argument('--output', type=pathlib.Path, required=True)
parser.add_argument('--wake-model', type=pathlib.Path, default=pathlib.Path(os.environ['APPDATA']) / 'whispertype/wake.rpw')
parser.add_argument('--seconds', type=float, default=15)
parser.add_argument('--warmup', type=float, default=3)
parser.add_argument('--trials', type=int, default=3)
parser.add_argument('--states', nargs='+', choices=['shortcut_idle', 'shortcut_recording',
    'wake_listening', 'wake_recording', 'idle_after_capture'],
    help='Optional subset for experiments; omit for the full comparison')
args = parser.parse_args()
assert sys.platform == 'win32'
assert 5 <= args.seconds <= 60 and 1 <= args.trials <= 10
binary = args.binary.resolve(strict=True)
assert args.wake_model.is_file(), 'A personal wake model is required'

user = c.WinDLL('user32', use_last_error=True)
kernel = c.WinDLL('kernel32', use_last_error=True)
psapi = c.WinDLL('psapi', use_last_error=True)
callback_type = c.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
user.EnumWindows.argtypes = [callback_type, w.LPARAM]
user.GetWindowThreadProcessId.argtypes = [w.HWND, c.POINTER(w.DWORD)]
user.GetClassNameW.argtypes = [w.HWND, w.LPWSTR, c.c_int]
user.PostMessageW.argtypes = [w.HWND, w.UINT, w.WPARAM, w.LPARAM]
kernel.GetProcessTimes.argtypes = [w.HANDLE] + [c.POINTER(w.FILETIME)] * 4
kernel.QueryProcessCycleTime.argtypes = [w.HANDLE, c.POINTER(c.c_ulonglong)]
kernel.GetProcessHandleCount.argtypes = [w.HANDLE, c.POINTER(w.DWORD)]

class Memory(c.Structure):
    _fields_ = [('cb', w.DWORD), ('faults', w.DWORD)] + [(name, c.c_size_t) for name in (
        'peak_working_set', 'working_set', 'peak_paged', 'paged', 'peak_nonpaged',
        'nonpaged', 'pagefile', 'peak_pagefile', 'private_bytes')]

psapi.GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(Memory), w.DWORD]

def snapshot(process):
    assert process.poll() is None, 'Profile process exited'
    handle = w.HANDLE(int(process._handle))
    times = [w.FILETIME() for _ in range(4)]
    assert kernel.GetProcessTimes(handle, *[c.byref(t) for t in times])
    cpu = sum((t.dwHighDateTime << 32) | t.dwLowDateTime for t in times[2:]) / 1e7
    memory = Memory()
    memory.cb = c.sizeof(memory)
    assert psapi.GetProcessMemoryInfo(handle, c.byref(memory), memory.cb)
    cycles = c.c_ulonglong()
    handles = w.DWORD()
    assert kernel.QueryProcessCycleTime(handle, c.byref(cycles))
    assert kernel.GetProcessHandleCount(handle, c.byref(handles))
    return dict(cpu_seconds=cpu, cpu_cycles=cycles.value,
                working_set_bytes=memory.working_set, private_bytes=memory.private_bytes,
                peak_working_set_bytes=memory.peak_working_set, handles=handles.value)

def window_for(pid):
    found = []
    @callback_type
    def visit(window, _):
        owner = w.DWORD()
        user.GetWindowThreadProcessId(window, c.byref(owner))
        if owner.value == pid:
            name = c.create_unicode_buffer(128)
            user.GetClassNameW(window, name, len(name))
            if name.value == 'Whispertype.Native.Tray.v1':
                found.append(window)
        return True
    user.EnumWindows(visit, 0)
    return found[0] if found else None

commands = {'start': 3, 'cancel': 5, 'wake-on': 6, 'wake-off': 7,
            'quit': 11, '_profile-wake-record': 20, '_profile-stats': 21}

def tail_stats(log):
    text = log.read_text(encoding='utf-8', errors='replace')
    # stderr writes can be observed mid-line; parse only complete JSON records.
    matches = re.findall(r'^Profile stats: (\{[^\n]+\})\r?\n', text, re.M)
    return [json.loads(line) for line in matches]

samples = []
traces = []
for trial in range(args.trials):
    with tempfile.TemporaryDirectory(prefix='whispertype-efficiency-') as folder:
        profile = pathlib.Path(folder).resolve()
        assert profile.parent == pathlib.Path(tempfile.gettempdir()).resolve()
        (profile / 'config.toml').write_text('shortcut = "CTRL + ALT + SHIFT + F24"\nmax_seconds = 120\n')
        shutil.copyfile(args.wake_model, profile / 'wake.rpw')
        env = dict(os.environ, WHISPERTYPE_HOME=str(profile), WHISPERTYPE_PROFILE='1', WHISPERTYPE_TRACE_AUDIO='1')
        env.pop('GROQ_API_KEY', None)
        log = profile / 'stderr.log'
        with log.open('wb') as stderr, (profile / 'stdout.log').open('wb') as stdout:
            process = subprocess.Popen([str(binary), 'run'], env=env, stdout=stdout,
                                       stderr=stderr, creationflags=subprocess.CREATE_NO_WINDOW)
            try:
                until = time.monotonic() + 10
                window = None
                while window is None and time.monotonic() < until:
                    assert process.poll() is None, log.read_text()
                    window = window_for(process.pid)
                    time.sleep(.02)
                assert window is not None, 'No profile control window'

                def command(name):
                    assert user.PostMessageW(window, 0x8004, commands[name], 0)

                def stats():
                    before = len(tail_stats(log))
                    command('_profile-stats')
                    until = time.monotonic() + 5
                    while time.monotonic() < until:
                        values = tail_stats(log)
                        if len(values) > before:
                            return values[-1]
                        assert process.poll() is None, log.read_text()
                        time.sleep(.01)
                    raise RuntimeError('Profile stats did not arrive')

                def measure(name, expected, wake_recording=False):
                    if args.states and name not in args.states:
                        return
                    time.sleep(args.warmup)
                    begin_state = stats()
                    assert begin_state['state'] == expected, (name, begin_state)
                    assert begin_state['wake_recording'] == wake_recording, (name, begin_state)
                    begin = snapshot(process)
                    started = time.perf_counter()
                    memory = []
                    while time.perf_counter() - started < args.seconds:
                        time.sleep(min(.5, max(.01, args.seconds - (time.perf_counter() - started))))
                        memory.append(snapshot(process))
                    ended = time.perf_counter()
                    end = snapshot(process)
                    end_state = stats()
                    assert end_state['state'] == expected, (name, end_state)
                    assert end_state['wake_recording'] == wake_recording, (name, end_state)
                    elapsed = ended - started
                    record = dict(trial=trial + 1, state=name, elapsed_seconds=elapsed,
                        cpu_percent_one_core=100*(end['cpu_seconds']-begin['cpu_seconds'])/elapsed,
                        cycles_per_second=(end['cpu_cycles']-begin['cpu_cycles'])/elapsed,
                        private_bytes=statistics.median(p['private_bytes'] for p in memory),
                        working_set_bytes=statistics.median(p['working_set_bytes'] for p in memory),
                        peak_working_set_bytes=max(p['working_set_bytes'] for p in memory),
                        handles=statistics.median(p['handles'] for p in memory),
                        audio_events_per_second=(end_state['audio_events']-begin_state['audio_events'])/elapsed,
                        recording_capacity_bytes=end_state['recording_capacity']*2)
                    samples.append(record)
                    print(json.dumps(record), flush=True)

                measure('shortcut_idle', 'idle')
                command('start')
                measure('shortcut_recording', 'recording')
                command('cancel')
                command('wake-on')
                measure('wake_listening', 'listening')
                command('_profile-wake-record')
                measure('wake_recording', 'recording', True)
                command('cancel')
                command('wake-off')
                measure('idle_after_capture', 'idle')
                command('quit')
                process.wait(timeout=8)
                trace = log.read_text(encoding='utf-8', errors='replace')
                assert 'Audio capture:' in trace
                assert not re.search(r'fell behind|Microphone:|Unknown command|Profile state: transcribing', trace), trace
                traces.append('\n'.join(line for line in trace.splitlines() if line.startswith('Audio capture:')))
            finally:
                if process.poll() is None:
                    process.kill()
                process.wait(timeout=5)

summary = {}
for state in dict.fromkeys(p['state'] for p in samples):
    rows = [p for p in samples if p['state'] == state]
    fields = ('cpu_percent_one_core', 'cycles_per_second', 'private_bytes',
              'working_set_bytes', 'peak_working_set_bytes', 'audio_events_per_second', 'handles')
    summary[state] = {key: statistics.median(p[key] for p in rows) for key in fields}
    summary[state]['cpu_range'] = [min(p['cpu_percent_one_core'] for p in rows), max(p['cpu_percent_one_core'] for p in rows)]
with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, r'HARDWARE\DESCRIPTION\System\CentralProcessor\0') as key:
    cpu = winreg.QueryValueEx(key, 'ProcessorNameString')[0].strip()
report = dict(label=args.label, os=platform.platform(), cpu=cpu, logical_cpus=os.cpu_count(),
    executable_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(), executable_bytes=binary.stat().st_size,
    sample_seconds=args.seconds, warmup_seconds=args.warmup, trials=args.trials,
    methodology='Same compiler/settings and common profiling hooks. Production capture/recording/detector paths; only automatic submission is suppressed. App-only, excluding Windows audio services and any other installed instance. Ambient room; no artificial working-set trimming. CPU is percent of one logical core; cycles are frequency-dependent. No audio saved or uploaded.',
    audio_trace=traces, summary=summary, samples=samples)
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
print('Saved '+str(args.output), flush=True)
